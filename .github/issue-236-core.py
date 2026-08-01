from pathlib import Path
import re


def read(path):
    return Path(path).read_text()


def write(path, text):
    Path(path).write_text(text)


def exact(path, old, new, expected=1):
    text = read(path)
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected}, found {count}: {old[:100]!r}")
    write(path, text.replace(old, new))


def regex(path, pattern, replacement, expected=1):
    text = read(path)
    new, count = re.subn(pattern, replacement, text, flags=re.S)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected} regex matches, found {count}")
    write(path, new)


write(
    "crates/dawn-core/src/transit.rs",
    '''//! Cross-Sector Transit domain state.
//!
//! `TransitHandoffState` is the single Ship-state representation carried by
//! Raft Commit payloads and persisted in `SectorTransitCompleted` for replay.
//! Source-local coordinates, anchors, and tackle state are deliberately absent.

use crate::fitting::FittingSnapshot;
use crate::item::ItemId;
use crate::{ShipId, ShipTypeId, Velocity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Runtime state that follows one Ship across a Sector ownership boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitHandoffState {
    pub ship_id: ShipId,
    pub ship_type_id: ShipTypeId,
    pub velocity: Velocity,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}
''',
)

exact("crates/dawn-core/src/lib.rs", "pub mod tick;\n", "pub mod tick;\npub mod transit;\n")
exact(
    "crates/dawn-core/src/lib.rs",
    "pub use tick::Tick;",
    "pub use tick::Tick;\npub use transit::TransitHandoffState;",
)

exact(
    "crates/dawn-core/src/events.rs",
    "            Self::SectorTransitCompleted(e) => e.ship_id,\n",
    "            Self::SectorTransitCompleted(e) => e.handoff.ship_id,\n",
)
regex(
    "crates/dawn-core/src/events.rs",
    r"/// A Sector Transit completed; ownership of `ship_id` moved from `from` to `to`\..*?(?=/// A committed Sector Transit was aborted)",
    '''/// A Sector Transit completed; ownership of `handoff.ship_id` moved from
/// `from` to `to`.
///
/// `handoff` is the same Transit-owned state carried by the consensus Commit,
/// so destination snapshot-plus-tail replay can materialize the Ship without
/// an in-memory Raft actor. Persistence `ShipSnapshot` does not cross this
/// protocol boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorTransitCompleted {
    pub handoff: crate::transit::TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    /// Source-local identity of the Request this completion closes.
    pub request_tick: Tick,
    /// Authoritative destination-Sector entry position.
    pub entry_pos: AbsolutePosition,
    /// Tick local to the EventStore that appended this record.
    pub tick: Tick,
}

''',
)
exact(
    "crates/dawn-core/src/events.rs",
    '''        let event = DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            ship_id: id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick::ZERO,
            entry_pos: AbsolutePosition::new(100.0, 0.0, 0.0),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick: Tick(8),
            ship_state: TransitShipState {
                ship_type_id: ShipTypeId(1),
                current_shield: 100.0,
                current_armor: 100.0,
                current_hull: 100.0,
                is_destroyed: false,
                capacitor: Some(50.0),
                fitting: FittingSnapshot::empty(),
                inventory: std::collections::BTreeMap::new(),
            },
        });
''',
    '''        let event = DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            handoff: crate::TransitHandoffState {
                ship_id: id,
                ship_type_id: ShipTypeId(1),
                velocity: Velocity::new(1.0, 0.0, 0.0),
                current_shield: 100.0,
                current_armor: 100.0,
                current_hull: 100.0,
                is_destroyed: false,
                capacitor: Some(50.0),
                fitting: FittingSnapshot::empty(),
                inventory: std::collections::BTreeMap::new(),
            },
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick::ZERO,
            entry_pos: AbsolutePosition::new(100.0, 0.0, 0.0),
            tick: Tick(8),
        });
''',
)

exact("crates/dawn-sector/src/transit.rs", "use crate::persistence::ShipSnapshot;\n", "")
exact(
    "crates/dawn-sector/src/transit.rs",
    "use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick};",
    "use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick, TransitHandoffState};",
)
exact(
    "crates/dawn-sector/src/transit.rs",
    '''    Commit {
        ship: Box<ShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        entry_pos_abs: AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    },
    Ack {
        ship: Box<ShipSnapshot>,
        from: SectorId,
        to: SectorId,
        entry_pos_abs: AbsolutePosition,
        request_tick: Tick,
    },
''',
    '''    Commit {
        handoff: Box<TransitHandoffState>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        entry_pos_abs: AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    },
    Ack {
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: Tick,
    },
''',
)
exact(
    "crates/dawn-sector/src/transit.rs",
    "            ship: Box::new(proposal.ship),\n",
    "            handoff: Box::new(proposal.handoff),\n",
)
exact(
    "crates/dawn-sector/src/transit.rs",
    '''        TransitOp::Ack {
            ship: Box::new(proposal.ship),
            from: proposal.from,
            to: proposal.to,
            entry_pos_abs: proposal.entry_pos_abs,
            request_tick: proposal.request_tick,
        }
''',
    '''        TransitOp::Ack {
            ship_id: proposal.ship_id,
            from: proposal.from,
            to: proposal.to,
            request_tick: proposal.request_tick,
        }
''',
)
exact(
    "crates/dawn-sector/src/transit.rs",
    '''            TransitOp::Commit {
                ship,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
                request_tick,
            } => {
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &ship,
                    from,
                    to,
                    entry_pos,
                    entry_pos_abs,
                    gate_id,
                    request_tick,
                ) {
                    propose_ack(raft, proposal);
                }
            }
            TransitOp::Ack {
                ship,
                from,
                to,
                entry_pos_abs,
                request_tick,
            } => {
                pipeline::apply_ack(node, &ship, from, to, entry_pos_abs, request_tick);
            }
''',
    '''            TransitOp::Commit {
                handoff,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
                request_tick,
            } => {
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &handoff,
                    from,
                    to,
                    entry_pos,
                    entry_pos_abs,
                    gate_id,
                    request_tick,
                ) {
                    propose_ack(raft, proposal);
                }
            }
            TransitOp::Ack {
                ship_id,
                from,
                to,
                request_tick,
            } => {
                pipeline::apply_ack(node, ship_id, from, to, request_tick);
            }
''',
)

exact("crates/dawn-sector/src/transit/pipeline.rs", "use crate::persistence::ShipSnapshot;\n", "")
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick};",
    "use dawn_core::{AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick, TransitHandoffState};",
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    '''pub(crate) struct CommitProposal {
    pub ship: ShipSnapshot,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: Position,
    pub entry_pos_abs: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(crate) struct AckProposal {
    pub ship: ShipSnapshot,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos_abs: AbsolutePosition,
    pub request_tick: Tick,
}
''',
    '''pub(crate) struct CommitProposal {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: Position,
    pub entry_pos_abs: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

#[derive(Debug)]
pub(crate) struct AckProposal {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}
''',
)
regex(
    "crates/dawn-sector/src/transit/pipeline.rs",
    r"fn snapshot_ship<S: EventStore>\(node: &SimulationNode<S>, ship_id: ShipId\) -> Option<ShipSnapshot> \{.*?\n\}\n\n",
    "",
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    '''    let Some(frozen) = node.snapshot_for_transit(ship_id) else {
        return false;
    };

    node.complete_outgoing_transit(
        &frozen,
        pending.to,
        pending.entry_pos_abs,
        pending.request_tick,
    );
''',
    '''    node.complete_outgoing_transit(
        ship_id,
        pending.to,
        pending.entry_pos_abs,
        pending.request_tick,
    );
''',
)
regex(
    "crates/dawn-sector/src/transit/pipeline.rs",
    r"fn request_matches<S: EventStore>\(.*?\n\}\n\n/// Apply a committed Request",
    '''fn matching_request<S: EventStore>(
    node: &SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> Option<PendingTransit> {
    if node.get_ship_position(ship_id).is_none() {
        return None;
    }
    pending_outgoing_transits(node).into_iter().find(|pending| {
        pending.ship_id == ship_id
            && pending.from == from
            && pending.to == to
            && pending.request_tick == request_tick
    })
}

/// Apply a committed Request''',
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    '''    Some(CommitProposal {
        ship: *data.ship,
        from: node.sector_id(),
''',
    '''    Some(CommitProposal {
        handoff: *data.handoff,
        from: node.sector_id(),
''',
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "    ship: &ShipSnapshot,\n",
    "    handoff: &TransitHandoffState,\n",
)
path = "crates/dawn-sector/src/transit/pipeline.rs"
text = read(path)
start = text.index("pub(crate) fn apply_commit")
end = text.index("/// Validate a committed Ack", start)
segment = text[start:end]
segment = segment.replace("ship.ship_id", "handoff.ship_id")
segment = segment.replace("node.handle_transit_commit(ship,", "node.handle_transit_commit(handoff,")
segment = segment.replace(
    '''    Some(AckProposal {
        ship: snapshot_ship(node, handoff.ship_id).unwrap_or_else(|| ship.clone()),
        from,
        to,
        entry_pos_abs,
        request_tick,
    })
''',
    '''    Some(AckProposal {
        ship_id: handoff.ship_id,
        from,
        to,
        request_tick,
    })
''',
)
write(path, text[:start] + segment + text[end:])
regex(
    "crates/dawn-sector/src/transit/pipeline.rs",
    r"pub\(crate\) fn apply_ack<S: EventStore>\(.*?\n\}\n\n/// Return only retry Commit proposals",
    '''pub(crate) fn apply_ack<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    if from != node.sector_id() {
        return false;
    }
    let Some(pending) = matching_request(node, ship_id, from, to, request_tick) else {
        return false;
    };
    node.complete_outgoing_transit(
        ship_id,
        pending.to,
        pending.entry_pos_abs,
        pending.request_tick,
    );
    true
}

/// Return only retry Commit proposals''',
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "        let Some(ship) = node.snapshot_for_transit(transit.ship_id) else {\n",
    "        let Some(handoff) = node.handoff_for_transit(transit.ship_id) else {\n",
)
exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "            ship,\n            from: transit.from,\n",
    "            handoff,\n            from: transit.from,\n",
)

exact(
    "crates/dawn-sector/src/node/transit.rs",
    "    DawnError, DomainEvent, JumpGateId, Position, SectorId, ShipId, ShipTypeId, Tick,\n",
    "    DawnError, DomainEvent, JumpGateId, Position, SectorId, ShipId, ShipTypeId, Tick,\n    TransitHandoffState,\n",
)
exact(
    "crates/dawn-sector/src/node/transit.rs",
    "    components::{CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, VelocityComp},\n",
    "    components::{CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, ShipStatsComp, VelocityComp},\n",
)
exact(
    "crates/dawn-sector/src/node/transit.rs",
    "use crate::persistence::{CompletedIncomingTransit, ShipSnapshot};\n",
    "use crate::persistence::CompletedIncomingTransit;\n",
)
regex(
    "crates/dawn-sector/src/node/transit.rs",
    r"impl ShipSnapshot \{.*?(?=/// Everything the Raft layer needs)",
    "",
)
exact(
    "crates/dawn-sector/src/node/transit.rs",
    '''/// Everything the Raft layer needs to propose a `TransitOp::Commit`, produced
/// by [`SimulationNode::prepare_transit_commit`]. `ship` is boxed for the same
/// reason `TransitOp::Commit` boxes it (ADR-0032 grew `ShipSnapshot` with
/// `inventory`).
#[derive(Debug)]
pub struct TransitCommitData {
    pub ship: Box<ShipSnapshot>,
''',
    '''/// Everything the Raft layer needs to propose a `TransitOp::Commit`, produced
/// by [`SimulationNode::prepare_transit_commit`].
#[derive(Debug)]
pub struct TransitCommitData {
    pub handoff: Box<TransitHandoffState>,
''',
)
exact(
    "crates/dawn-sector/src/node/transit.rs",
    '''        let ship = self.snapshot_for_transit(ship_id)?;
        Some(TransitCommitData {
            ship: Box::new(ship),
''',
    '''        let handoff = self.handoff_for_transit(ship_id)?;
        Some(TransitCommitData {
            handoff: Box::new(handoff),
''',
)
regex(
    "crates/dawn-sector/src/node/transit.rs",
    r"    /// Read-only export for the `TransitOp::Commit` proposal:.*?(?=    pub\(crate\) fn transit_commit_retry_due)",
    '''    /// Read-only export for the `TransitOp::Commit` proposal.
    #[cfg(test)]
    fn export_transit(&self, ship_id: ShipId) -> Option<TransitHandoffState> {
        self.handoff_for_transit(ship_id)
    }

    pub(crate) fn handoff_for_transit(
        &self,
        ship_id: ShipId,
    ) -> Option<TransitHandoffState> {
        let &entity = self.ships.index.get(&ship_id)?;
        if !self.world.transit_state(entity).is_in_transit() {
            return None;
        }
        let velocity = self.world.get::<VelocityComp>(entity)?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.world.get::<HullComp>(entity)?;
            (hull.shield(), hull.armor(), hull.hull(), hull.is_destroyed())
        };
        let capacitor = self.world.get::<CapacitorComp>(entity).map(|c| c.current);
        let fitting = self
            .world
            .get::<FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(FittingSnapshot::empty);
        let ship_type_id = self
            .ships
            .type_ids
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipTypeId(0));
        let inventory = self
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();
        Some(TransitHandoffState {
            ship_id,
            ship_type_id,
            velocity,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            inventory,
        })
    }

''',
)
regex(
    "crates/dawn-sector/src/node/transit.rs",
    r"    pub fn complete_outgoing_transit\(.*?(?=    /// Complete an incoming Sector Transit)",
    '''    pub fn complete_outgoing_transit(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        let Some(handoff) = self.handoff_for_transit(ship_id) else {
            return;
        };
        let Some(event) =
            self.complete_outgoing_state(&handoff, to, entry_pos_abs, request_tick)
        else {
            return;
        };
        self.event_store.append(event);
    }

    fn complete_outgoing_state(
        &mut self,
        handoff: &TransitHandoffState,
        to: SectorId,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) -> Option<DomainEvent> {
        if !self.ships.index.contains_key(&handoff.ship_id) {
            return None;
        }
        self.remove_ship(handoff.ship_id);
        Some(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                handoff: handoff.clone(),
                from: self.sector_id,
                to,
                request_tick,
                entry_pos: entry_pos_abs,
                tick: self.current_tick,
            },
        ))
    }

''',
)
regex(
    "crates/dawn-sector/src/node/transit.rs",
    r"    /// Complete an incoming Sector Transit:.*?(?=    /// Re-anchor a Ship that just arrived)",
    '''    /// Complete an incoming Sector Transit from canonical handoff state.
    fn import_transit(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        for event in self.materialize_incoming_state(
            handoff,
            from,
            entry_pos,
            entry_pos_abs,
            request_tick,
        ) {
            self.event_store.append(event);
        }
    }

    /// The single mapping from Transit handoff state into destination ECS.
    fn restore_ship_from_handoff(
        &mut self,
        handoff: &TransitHandoffState,
        entry_pos: Position,
    ) {
        self.insert_to_world(handoff.ship_id, entry_pos, handoff.velocity);
        self.ships.type_ids.insert(handoff.ship_id, handoff.ship_type_id);
        let base = self
            .ship_type_registry
            .get(&handoff.ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);
        self.base_stats.insert(handoff.ship_id, base);
        if let Some(&entity) = self.ships.index.get(&handoff.ship_id) {
            self.world.set_ship_stats(entity, base);
            let fitting = FittingComp::from_snapshot(&handoff.fitting, &self.module_registry);
            let _ = self.world.insert_one(entity, fitting);
            self.reapply_fitting(handoff.ship_id);
            if let Some(mut hull) = self.world.get_mut::<HullComp>(entity) {
                hull.set_hp(
                    handoff.current_shield,
                    handoff.current_armor,
                    handoff.current_hull,
                );
            }
            if let Some(current) = handoff.capacitor {
                let _ = self.world.insert_one(entity, CapacitorComp { current });
            }
            let _ = self.world.insert_one(
                entity,
                InventoryComp {
                    items: handoff.inventory.clone(),
                },
            );
        }
    }

    fn materialize_incoming_state(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) -> Vec<DomainEvent> {
        self.restore_ship_from_handoff(handoff, entry_pos);
        let mut events = Vec::with_capacity(2);
        if let Some(event) = self.rebase_after_transit(handoff.ship_id, entry_pos_abs) {
            events.push(event);
        }
        events.push(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                handoff: handoff.clone(),
                from,
                to: self.sector_id,
                request_tick,
                entry_pos: entry_pos_abs,
                tick: self.current_tick,
            },
        ));
        events
    }

    pub fn handle_transit_commit(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    ) {
        let ship_id = handoff.ship_id;
        self.record_completed_incoming_transit(ship_id, from, self.sector_id, request_tick);
        self.import_transit(handoff, from, entry_pos, entry_pos_abs, request_tick);
        if let Some(gate_id) = gate_id {
            let to = self.sector_id();
            self.append_jump_events(ship_id, gate_id, from, to, entry_pos_abs);
        }
    }

''',
)
exact(
    "crates/dawn-sector/src/node/transit.rs",
    '''        if self.sector_id == e.from {
            self.remove_ship(e.ship_id);
        } else if self.sector_id == e.to {
            self.record_completed_incoming_transit(e.ship_id, e.from, e.to, e.request_tick);
            if self.ships.index.contains_key(&e.ship_id) {
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
                return;
            }
            let entry_pos = Position::new(e.entry_pos[0], e.entry_pos[1], e.entry_pos[2]);
            let snapshot =
                ship_snapshot_from_transit(e.ship_id, &e.ship_state, entry_pos, e.velocity);
            self.restore_ship_from_snapshot(&snapshot);
            self.rebase_ship_anchor_state(e.ship_id, e.entry_pos);
        }
''',
    '''        if self.sector_id == e.from {
            self.remove_ship(e.handoff.ship_id);
        } else if self.sector_id == e.to {
            self.record_completed_incoming_transit(
                e.handoff.ship_id,
                e.from,
                e.to,
                e.request_tick,
            );
            if self.ships.index.contains_key(&e.handoff.ship_id) {
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
                return;
            }
            let entry_pos = Position::new(e.entry_pos[0], e.entry_pos[1], e.entry_pos[2]);
            self.restore_ship_from_handoff(&e.handoff, entry_pos);
            self.rebase_ship_anchor_state(e.handoff.ship_id, e.entry_pos);
        }
''',
)

for path in [
    "crates/dawn-sector/src/transit/tests.rs",
    "crates/dawn-sector/src/transit/pipeline.rs",
    "crates/dawn-sector/src/node/transit.rs",
]:
    text = read(path)
    text = text.replace("snapshot_for_transit", "handoff_for_transit")
    text = text.replace("data.ship", "data.handoff")
    text = text.replace("proposal.ship", "proposal.handoff")
    text = text.replace("outbound.ship", "outbound.handoff")
    text = text.replace("returning.ship", "returning.handoff")
    write(path, text)
