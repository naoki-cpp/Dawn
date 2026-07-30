//! Ship ownership and identity maps for one `SimulationNode`.
//!
//! `ShipRegistry` groups the four HashMaps that track which ships exist and
//! who owns them. All fields are `pub(super)` so every `node::*` submodule
//! can still read them directly, but removal goes through `remove()` so the
//! "a ship exists iff it's in all four maps, and its ECS entity is despawned"
//! invariant has exactly one owner instead of being re-implemented at every
//! removal site.
//!
//! # Owned ship vs. active ship (ADR-0037)
//!
//! `owners` is the ownership source of truth — `ShipId` → `PlayerId`, and
//! nothing here stops one `PlayerId` from owning several ships. `active_ship`
//! is a separate, narrower concept: which one of a player's owned ships is
//! currently routable (flight commands, module activation, Undock — see
//! `SimulationNode::is_active_ship`). The two coincide 1:1 today because
//! nothing yet grants a second owned ship, but the split exists so that
//! changes when it does (docs/architecture/ownership.md §7).

use std::collections::HashMap;

use dawn_core::{PlayerId, ShipId, ShipTypeId};
use dawn_ecs::{Entity, SimWorld};

/// Ship identity and ownership bookkeeping for one Sector node.
pub(super) struct ShipRegistry {
    /// Maps `ShipId` → hecs `Entity` for O(1) ECS lookups.
    pub(super) index: HashMap<ShipId, Entity>,
    /// `ShipId` → `ShipTypeId` (used for `ship_type_name` in `InitialState`).
    pub(super) type_ids: HashMap<ShipId, ShipTypeId>,
    /// `PlayerId` → `ShipId` of the ship currently routable for that player
    /// (flight commands / module activation / Undock — ADR-0037). Not the
    /// full ownership roster; see `owners` for that.
    pub(super) active_ship: HashMap<PlayerId, ShipId>,
    /// `ShipId` → `PlayerId` (reverse ownership lookup — every ship a player
    /// owns, not just the active one).
    pub(super) owners: HashMap<ShipId, PlayerId>,
}

impl ShipRegistry {
    pub(super) fn new() -> Self {
        Self {
            index: HashMap::new(),
            type_ids: HashMap::new(),
            active_ship: HashMap::new(),
            owners: HashMap::new(),
        }
    }

    /// Removes a ship from every identity/ownership map and despawns its ECS
    /// entity, in one place. Returns the despawned `Entity`, or `None` if
    /// `ship_id` was already gone.
    ///
    /// Before this existed, callers hand-rolled a 4-6 line removal sequence
    /// at each despawn site (combat death, ShipDespawned replay, Sector
    /// Transit departure) and one of them (`transit.rs`) silently
    /// forgot `owners`/`active_ship`, leaving a dangling ownership entry for
    /// a transited ship. Routing every removal through here makes that class
    /// of bug structurally impossible.
    ///
    /// Only clears `active_ship` if the removed ship *was* that player's
    /// active ship — a player who owns a second ship elsewhere must not have
    /// their active pointer silently pulled out from under them because some
    /// other owned ship was removed (ADR-0037).
    pub(super) fn remove(&mut self, ship_id: ShipId, world: &mut SimWorld) -> Option<Entity> {
        let entity = self.index.remove(&ship_id)?;
        world.despawn_ship(entity);
        self.type_ids.remove(&ship_id);
        if let Some(player_id) = self.owners.remove(&ship_id) {
            if self.active_ship.get(&player_id) == Some(&ship_id) {
                self.active_ship.remove(&player_id);
            }
        }
        Some(entity)
    }
}
