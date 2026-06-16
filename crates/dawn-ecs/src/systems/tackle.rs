//! Tackle System — Step 4.5 (after Capacitor, before Lock). ADR-0024.
//!
//! This module exposes types used by `SimulationNode::process_tackle()`.
//! The system logic lives in `node.rs` because it needs `ship_index`
//! (ShipId → hecs::Entity) which is owned by the node, not `SimWorld`.

use dawn_core::{DomainEvent, ShipId, Tick};
use dawn_core::events::{TackleApplied, TackleReleased};

pub struct TackleResult {
    pub events: Vec<DomainEvent>,
}

impl TackleResult {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn push_applied(&mut self, ship_id: ShipId, by: ShipId, tick: Tick) {
        self.events.push(DomainEvent::TackleApplied(TackleApplied { ship_id, by, tick }));
    }

    pub fn push_released(&mut self, ship_id: ShipId, by: ShipId, tick: Tick) {
        self.events.push(DomainEvent::TackleReleased(TackleReleased { ship_id, by, tick }));
    }
}
