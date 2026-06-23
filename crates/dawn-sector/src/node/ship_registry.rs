//! Ship ownership and identity maps for one `SimulationNode`.
//!
//! `ShipRegistry` groups the four HashMaps that track which ships exist and
//! who owns them. All fields are `pub(super)` so every `node::*` submodule
//! can access them directly.

use std::collections::HashMap;

use dawn_core::{PlayerId, ShipId, ShipTypeId};
use dawn_ecs::Entity;

/// Ship identity and ownership bookkeeping for one Sector node.
pub(super) struct ShipRegistry {
    /// Maps `ShipId` → hecs `Entity` for O(1) ECS lookups.
    pub(super) index: HashMap<ShipId, Entity>,
    /// `ShipId` → `ShipTypeId` (used for `ship_type_name` in `InitialState`).
    pub(super) type_ids: HashMap<ShipId, ShipTypeId>,
    /// `PlayerId` → `ShipId` (player-ship ownership, forward lookup).
    pub(super) by_player: HashMap<PlayerId, ShipId>,
    /// `ShipId` → `PlayerId` (reverse ownership lookup).
    pub(super) owners: HashMap<ShipId, PlayerId>,
}

impl ShipRegistry {
    pub(super) fn new() -> Self {
        Self {
            index: HashMap::new(),
            type_ids: HashMap::new(),
            by_player: HashMap::new(),
            owners: HashMap::new(),
        }
    }
}
