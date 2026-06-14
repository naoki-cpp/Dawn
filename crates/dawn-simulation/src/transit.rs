//! Sector Transit proposals carried through the Raft Log (ADR-0014 §3).
//!
//! `TransitOp` is the *Command* (proposal) type serialized into Raft
//! `LogEntry` payloads — never an Event (INV-006). Once an op commits,
//! every node applies it deterministically:
//!
//! - `Request`: the owning (from) node marks the Ship `InTransit`, appends
//!   `SectorTransitRequested`, exports the Ship's state, and proposes a
//!   follow-up `Commit` op carrying that state (ADR-0014 §3 [4]).
//! - `Commit`: the destination (to) node imports the Ship at `entry_pos`
//!   and appends `SectorTransitCompleted`. Other nodes ignore it.

use crate::node::SimulationNode;
use crate::star_map;
use crate::snapshot::ShipSnapshot;
use dawn_consensus::RaftActorHandle;
use dawn_core::{JumpGateId, Position, SectorId, ShipId};
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
        to     : SectorId,
        gate_id: Option<JumpGateId>,
    },
    /// Stage 2: the from-node ships the exported state to the to-node.
    Commit {
        ship     : ShipSnapshot,
        from     : SectorId,
        to       : SectorId,
        entry_pos: Position,
        gate_id  : Option<JumpGateId>,
    },
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
        postcard::from_bytes(payload).ok()
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
pub(crate) fn apply_committed_raft_entries<S: EventStore>(
    node        : &mut SimulationNode<S>,
    raft        : &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Ok(payload) = committed_rx.try_recv() {
        let Some(op) = TransitOp::decode(&payload) else { continue };
        match op {
            TransitOp::Request { ship_id, to, gate_id } => {
                let cmd = dawn_core::commands::TransitCommand { ship_id, to };
                if node.propose_transit(cmd).is_ok() {
                    // This node owned the Ship: hand its state to the
                    // destination through a second Raft round.
                    //
                    // For Jump Gate transits, spawn the Ship near the gate in
                    // the destination Sector that leads back to `from`, so
                    // the player can immediately jump back (ADR-0009).
                    let entry_pos = gate_id
                        .and_then(|_| {
                            star_map::gates_in_sector(to)
                                .into_iter()
                                .find(|g| g.to_sector == node.sector_id())
                                .map(|g| g.position)
                        })
                        .unwrap_or(Position::ORIGIN);
                    if let Some(ship) = node.export_transit(ship_id, entry_pos) {
                        let from = node.sector_id();
                        raft.propose(
                            TransitOp::Commit { ship, from, to, entry_pos, gate_id }.encode(),
                        );
                    }
                }
            }
            TransitOp::Commit { ship, from, to, entry_pos, gate_id } => {
                if to == node.sector_id() {
                    let ship_id = ship.ship_id;
                    node.import_transit(&ship, from, entry_pos);
                    if let Some(gate_id) = gate_id {
                        node.append_jump_events(ship_id, gate_id, from, to, entry_pos);
                    }
                }
            }
        }
    }
}

/// Advance one cluster node by one logical Tick in the canonical step order
/// (ADR-0014): Step 7.5 (apply committed Raft entries) → simulation tick →
/// Step 10 (advance this node's Raft election/heartbeat timers).
///
/// Shared by the `--serve --cluster` warm-up and main loops so the per-node
/// step order has a single source of truth. The actor path
/// (`SectorSimulatorActor`) keeps its own variant because it interleaves a
/// ReplicationBus flush (Step 9) between the tick and the Raft timer step.
pub(crate) fn step_cluster_node<S: EventStore>(
    node         : &mut SimulationNode<S>,
    raft         : &RaftActorHandle,
    committed_rx : &mut mpsc::UnboundedReceiver<Vec<u8>>,
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
    use dawn_core::{NodeId, ShipTypeId, Velocity};

    #[test]
    fn request_op_round_trips_through_encode_and_decode() {
        let op = TransitOp::Request {
            ship_id: ShipId::new(NodeId(0), 42),
            to     : SectorId(1),
            gate_id: None,
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Request { ship_id, to, gate_id } => {
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
            to     : SectorId(1),
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
            ship: ShipSnapshot {
                ship_id       : ShipId::new(NodeId(0), 7),
                ship_type_id  : ShipTypeId(1),
                position      : Position::new(1.0, 2.0, 3.0),
                velocity      : Velocity::new(4.0, 5.0, 6.0),
                current_shield: 10.0,
                current_armor : 20.0,
                current_hull  : 30.0,
                is_destroyed  : false,
                capacitor     : Some(50.0),
                fitting       : FittingSnapshot::empty(),
            },
            from     : SectorId(0),
            to       : SectorId(1),
            entry_pos: Position::new(500.0, 0.0, 0.0),
            gate_id  : None,
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Commit { ship, from, to, entry_pos, gate_id } => {
                assert_eq!(ship.ship_id, ShipId::new(NodeId(0), 7));
                assert_eq!(ship.capacitor, Some(50.0));
                assert_eq!(from, SectorId(0));
                assert_eq!(to, SectorId(1));
                assert_eq!(entry_pos, Position::new(500.0, 0.0, 0.0));
                assert_eq!(gate_id, None);
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn decode_returns_none_for_garbage_payload() {
        assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
    }
}
