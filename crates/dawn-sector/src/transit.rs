//! Sector Transit proposals carried through the Raft Log (ADR-0014 §3).
//!
//! `TransitOp` is the *Command* (proposal) type serialized into Raft
//! `LogEntry` payloads — never an Event (INV-006). Once an op commits,
//! every node applies it deterministically:
//!
//! - `Request`: the owning (from) node marks the Ship `InTransit`, appends
//!   `SectorTransitRequested`, exports the Ship's state, and proposes a
//!   follow-up `Commit` op carrying that state (ADR-0014 §3 \[4\]).
//! - `Commit`: the destination (to) node imports the Ship at `entry_pos`
//!   and appends `SectorTransitCompleted`. Other nodes ignore it.

use crate::node::SimulationNode;
use crate::persistence::{snapshot::LegacyShipSnapshot, ShipSnapshot};
use dawn_consensus::RaftActorHandle;
use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A Sector Transit proposal as it travels through the Raft Log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitOp {
    /// Stage 1: a node requests moving `ship_id` to Sector `to`.
    ///
    /// `gate_id` is `Some` when this Transit was initiated via a Jump Gate
    /// (ADR-0009); `Commit` carries the same value so Step 7.5 can append
    /// `JumpGateUsed` on the destination node.
    Request {
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    },
    /// Stage 2: the from-node ships the exported state to the to-node.
    Commit {
        // Boxed (ADR-0032 grew ShipSnapshot with `inventory`, pushing this
        // variant well past Request's size): keeps every TransitOp the size
        // of the smallest variant instead of the largest.
        ship: Box<ShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        /// Precise f64 Sector-frame arrival point (ADR-0029): `entry_pos`
        /// alone is too coarse to re-anchor the Ship against at true-AU
        /// magnitudes (see `SimulationNode::import_transit`).
        entry_pos_abs: AbsolutePosition,
        gate_id: Option<JumpGateId>,
    },
}

/// Pre-ADR-0044 transit payload used to decode committed Raft entries written
/// before `ShipSnapshot::absolute_position` was added. Postcard is positional,
/// so decoding the legacy nested ship requires a matching legacy enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum LegacyTransitOp {
    Request {
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    },
    Commit {
        ship: Box<LegacyShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        entry_pos_abs: [f64; 3],
        gate_id: Option<JumpGateId>,
    },
}

impl From<LegacyTransitOp> for TransitOp {
    fn from(legacy: LegacyTransitOp) -> Self {
        match legacy {
            LegacyTransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => Self::Request {
                ship_id,
                to,
                gate_id,
            },
            LegacyTransitOp::Commit {
                ship,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
            } => Self::Commit {
                ship: Box::new((*ship).into()),
                from,
                to,
                entry_pos,
                entry_pos_abs: entry_pos_abs.into(),
                gate_id,
            },
        }
    }
}

impl TransitOp {
    /// Serialize for a Raft `LogEntry` payload.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("TransitOp serialization cannot fail")
    }

    /// Deserialize from a committed Raft `LogEntry` payload.
    ///
    /// Returns `None` for payloads that are not a `TransitOp` (future
    /// proposal types share the same log).
    pub fn decode(payload: &[u8]) -> Option<Self> {
        postcard::from_bytes(payload).ok().or_else(|| {
            postcard::from_bytes::<LegacyTransitOp>(payload)
                .ok()
                .map(Into::into)
        })
    }
}

/// Tick Step 7.5 (ADR-0014 §7): apply committed Raft Log entries to a node.
///
/// `Request`: if `node` owns the Ship, mark it `InTransit` (appends
/// `SectorTransitRequested`), export its state, and propose the follow-up
/// `Commit` op carrying the snapshot.
/// `Commit`: if `node` is the destination Sector, import the Ship at
/// `entry_pos` (appends `SectorTransitCompleted`), plus `JumpGateUsed` /
/// `StarSystemChanged` when the Transit came through a Jump Gate (ADR-0009).
///
/// Shared by `SectorSimulatorActor` and the `--serve --cluster` loop so the
/// Step 7.5 semantics cannot drift between the two call sites.
pub fn apply_committed_raft_entries<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Ok(payload) = committed_rx.try_recv() {
        let Some(op) = TransitOp::decode(&payload) else {
            continue;
        };
        match op {
            TransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => {
                // `prepare_transit_commit` owns the Gate-lookup/entry-point
                // logic (ADR-0009/0029) — this orchestrator just wraps the
                // result into the follow-up Raft proposal.
                if let Some(data) = node.prepare_transit_commit(ship_id, to, gate_id) {
                    let from = node.sector_id();
                    raft.propose(
                        TransitOp::Commit {
                            ship: data.ship,
                            from,
                            to,
                            entry_pos: data.entry_pos,
                            entry_pos_abs: data.entry_pos_abs,
                            gate_id,
                        }
                        .encode(),
                    );
                }
            }
            TransitOp::Commit {
                ship,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
            } => {
                if to == node.sector_id() {
                    node.handle_transit_commit(&ship, from, entry_pos, entry_pos_abs, gate_id);
                }
            }
        }
    }
}

/// Per-node runtime tick output needed by the outer runtime loops.
#[derive(Debug)]
pub struct RuntimeTickOutput {
    pub tick_result: crate::node::TickResult,
    pub events: Vec<DomainEvent>,
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub completed_warps: Vec<ShipId>,
}

/// Advance one cluster node by one logical Tick in the canonical runtime order
/// (ADR-0014): Step 7.5 committed Raft entries, simulation tick, caller hook
/// for durable-log consumers, Step 10 Raft timers, then transient tick outputs.
///
/// Shared by the actor and `--serve --cluster` loops so the ordering cannot
/// drift. The hook runs after Step 7.5 + simulation events are appended and
/// before `raft.tick()`, preserving the actor's replication-before-reply
/// contract while keeping the core step sequence in one place.
pub fn run_runtime_tick<S, F>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_appended: F,
) -> RuntimeTickOutput
where
    S: EventStore,
    F: FnOnce(&mut SimulationNode<S>, &crate::node::TickResult),
{
    let events_before = node.total_event_count() as u64;
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    after_events_appended(node, &result);
    raft.tick();
    let pending_auto_jumps = node.drain_pending_auto_jumps();
    let completed_warps = node.drain_completed_warps();
    let events = node
        .event_store()
        .iter_from(events_before)
        .map(|record| record.event.clone())
        .collect();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}

/// Runs a player-initiated jump attempt through its in-range/auto-warp/approach
/// fallback chain (`SimulationNode::apply_jump_with_fallback`, `node/jump.rs`)
/// and, if the ship was in range, proposes the resulting `TransitOp::Request`
/// to Raft. The fallback chain stays in `dawn-sector`; only the Raft proposal
/// lives here, since `RaftActorHandle` isn't available to `node/jump.rs`.
///
/// Shared by the single-sector, clustered, and production Node serve loops so
/// the proposal payload cannot drift between them. Callers still match on the
/// returned outcome to log it in their own format.
pub fn propose_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> crate::node::JumpOutcome {
    let outcome = node.apply_jump_with_fallback(ship_id, gate_id);
    if let crate::node::JumpOutcome::NeedsTransitProposal { to } = outcome {
        propose_transit_request(raft, ship_id, to, gate_id);
    }
    outcome
}

/// Proposes an auto-jump queued by a completed warp/approach fallback
/// (`SimulationNode::drain_pending_auto_jumps` + `resolve_auto_jump`), if the
/// ship is now in range of the gate. Returns the destination Sector when a
/// proposal was made.
pub fn propose_auto_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> Option<SectorId> {
    let to = node.resolve_auto_jump(ship_id, gate_id)?;
    propose_transit_request(raft, ship_id, to, gate_id);
    Some(to)
}

fn propose_transit_request(
    raft: &RaftActorHandle,
    ship_id: ShipId,
    to: SectorId,
    gate_id: JumpGateId,
) {
    raft.propose(
        TransitOp::Request {
            ship_id,
            to,
            gate_id: Some(gate_id),
        }
        .encode(),
    );
}

/// Advance one cluster node by one logical Tick in the canonical step order
/// when the caller owns all transient output drains.
pub fn step_cluster_node<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
) -> crate::node::TickResult {
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    raft.tick();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::fitting::FittingSnapshot;
    use dawn_core::{NodeId, Position, SectorBounds, ShipTypeId, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn raft_handle() -> (
        RaftActorHandle,
        mpsc::UnboundedReceiver<dawn_consensus::RaftActorMessage>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (RaftActorHandle::new(tx), rx)
    }

    fn decode_proposed_transit(
        rx: &mut mpsc::UnboundedReceiver<dawn_consensus::RaftActorMessage>,
    ) -> TransitOp {
        let msg = rx.try_recv().expect("a proposal must have been sent");
        let payload = match msg {
            dawn_consensus::RaftActorMessage::Propose(payload) => payload,
            other => panic!("expected Propose, got {other:?}"),
        };
        TransitOp::decode(&payload).expect("payload must decode as a TransitOp")
    }

    #[test]
    fn propose_jump_proposes_a_transit_request_when_the_ship_is_in_range() {
        let mut node = mem_node();
        let (raft, mut rx) = raft_handle();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        let near_gate_abs = [
            gate.abs_m[0] - (gate.activation_radius * 0.5),
            gate.abs_m[1],
            gate.abs_m[2],
        ];
        let player_id = node.next_player_id();
        let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, near_gate_abs);

        let outcome = propose_jump(&mut node, &raft, ship, JumpGateId(0));

        assert_eq!(
            outcome,
            crate::node::JumpOutcome::NeedsTransitProposal { to: gate.to_sector }
        );
        match decode_proposed_transit(&mut rx) {
            TransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => {
                assert_eq!(ship_id, ship);
                assert_eq!(to, gate.to_sector);
                assert_eq!(gate_id, Some(JumpGateId(0)));
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn propose_jump_does_not_propose_when_the_ship_is_out_of_range() {
        let mut node = mem_node();
        let (raft, mut rx) = raft_handle();
        let player_id = node.next_player_id();
        let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        let outcome = propose_jump(&mut node, &raft, ship, JumpGateId(0));

        assert!(
            !matches!(
                outcome,
                crate::node::JumpOutcome::NeedsTransitProposal { .. }
            ),
            "ship spawned at the origin must not already be in Gate 0's activation radius"
        );
        assert!(
            rx.try_recv().is_err(),
            "no Transit proposal for a ship still out of the gate's activation radius"
        );
    }

    #[test]
    fn request_op_round_trips_through_encode_and_decode() {
        let op = TransitOp::Request {
            ship_id: ShipId::new(NodeId(0), 42),
            to: SectorId(1),
            gate_id: None,
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => {
                assert_eq!(ship_id, ShipId::new(NodeId(0), 42));
                assert_eq!(to, SectorId(1));
                assert_eq!(gate_id, None);
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn request_op_round_trips_with_jump_gate_id() {
        let op = TransitOp::Request {
            ship_id: ShipId::new(NodeId(0), 42),
            to: SectorId(1),
            gate_id: Some(dawn_core::JumpGateId(0)),
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Request { gate_id, .. } => {
                assert_eq!(gate_id, Some(dawn_core::JumpGateId(0)));
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn commit_op_round_trips_with_full_ship_snapshot() {
        let op = TransitOp::Commit {
            ship: Box::new(ShipSnapshot {
                ship_id: ShipId::new(NodeId(0), 7),
                ship_type_id: ShipTypeId(1),
                absolute_position: None,
                position: Position::new(1.0, 2.0, 3.0),
                anchor: dawn_core::AnchorId(0),
                velocity: Velocity::new(4.0, 5.0, 6.0),
                current_shield: 10.0,
                current_armor: 20.0,
                current_hull: 30.0,
                is_destroyed: false,
                capacitor: Some(50.0),
                fitting: FittingSnapshot::empty(),
                tackled_by: vec![],
                inventory: std::collections::BTreeMap::new(),
            }),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: Position::new(500.0, 0.0, 0.0),
            entry_pos_abs: AbsolutePosition::new(500.0, 0.0, 0.0),
            gate_id: None,
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Commit {
                ship,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
            } => {
                assert_eq!(ship.ship_id, ShipId::new(NodeId(0), 7));
                assert_eq!(ship.capacitor, Some(50.0));
                assert_eq!(from, SectorId(0));
                assert_eq!(to, SectorId(1));
                assert_eq!(entry_pos, Position::new(500.0, 0.0, 0.0));
                assert_eq!(entry_pos_abs, AbsolutePosition::new(500.0, 0.0, 0.0));
                assert_eq!(gate_id, None);
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn decode_returns_none_for_garbage_payload() {
        assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
    }

    #[test]
    fn legacy_commit_payload_decodes_without_absolute_position() {
        let legacy = LegacyTransitOp::Commit {
            ship: Box::new(LegacyShipSnapshot {
                ship_id: ShipId::new(NodeId(0), 7),
                ship_type_id: ShipTypeId(1),
                position: Position::new(1.0, 2.0, 3.0),
                anchor: dawn_core::AnchorId(0),
                velocity: Velocity::new(4.0, 5.0, 6.0),
                current_shield: 10.0,
                current_armor: 20.0,
                current_hull: 30.0,
                is_destroyed: false,
                capacitor: Some(50.0),
                fitting: FittingSnapshot::empty(),
                tackled_by: vec![],
                inventory: std::collections::BTreeMap::new(),
            }),
            from: SectorId(0),
            to: SectorId(1),
            entry_pos: Position::new(500.0, 0.0, 0.0),
            entry_pos_abs: [500.0, 0.0, 0.0],
            gate_id: None,
        };
        let decoded = TransitOp::decode(&postcard::to_stdvec(&legacy).unwrap()).unwrap();

        match decoded {
            TransitOp::Commit {
                ship,
                entry_pos_abs,
                ..
            } => {
                assert_eq!(ship.ship_id, ShipId::new(NodeId(0), 7));
                assert_eq!(ship.absolute_position, None);
                assert_eq!(entry_pos_abs, AbsolutePosition::new(500.0, 0.0, 0.0));
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }
}
