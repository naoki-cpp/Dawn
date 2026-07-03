//! Ship ownership and identity maps for one `SimulationNode`.
//!
//! `ShipRegistry` groups the four HashMaps that track which ships exist and
//! who owns them. All fields are `pub(super)` so every `node::*` submodule
//! can still read them directly, but removal goes through `remove()` so the
//! "a ship exists iff it's in all four maps, and its ECS entity is despawned"
//! invariant has exactly one owner instead of being re-implemented at every
//! removal site.

use std::collections::HashMap;

use dawn_core::{PlayerId, ShipId, ShipTypeId};
use dawn_ecs::{Entity, SimWorld};

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

    /// Removes a ship from every identity/ownership map and despawns its ECS
    /// entity, in one place. Returns the despawned `Entity`, or `None` if
    /// `ship_id` was already gone.
    ///
    /// Before this existed, callers hand-rolled a 4-6 line removal sequence
    /// at each despawn site (combat death, ShipDespawned replay, Sector
    /// Transit departure) and one of them (`transit_flow.rs`) silently
    /// forgot `owners`/`by_player`, leaving a dangling ownership entry for a
    /// transited ship. Routing every removal through here makes that class
    /// of bug structurally impossible.
    pub(super) fn remove(&mut self, ship_id: ShipId, world: &mut SimWorld) -> Option<Entity> {
        let entity = self.index.remove(&ship_id)?;
        world.despawn_ship(entity);
        self.type_ids.remove(&ship_id);
        if let Some(player_id) = self.owners.remove(&ship_id) {
            self.by_player.remove(&player_id);
        }
        Some(entity)
    }
}
