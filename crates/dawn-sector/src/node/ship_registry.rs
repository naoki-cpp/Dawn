//! Ship identity maps for one `SimulationNode`.
//!
//! Ownership and active-ship routing belong to `PlayerState`; this registry
//! only owns the ECS identity/type indexes. Keeping those maps separate makes
//! player authority explicit instead of treating it as a property of a ship
//! lookup table.

use std::collections::HashMap;

use dawn_core::{ShipId, ShipTypeId};
use dawn_ecs::{Entity, SimWorld};

/// Ship identity bookkeeping for one Sector node.
pub(super) struct ShipRegistry {
    /// Maps `ShipId` to hecs `Entity` for O(1) ECS lookups.
    pub(super) index: HashMap<ShipId, Entity>,
    /// Maps `ShipId` to `ShipTypeId` for `InitialState` projections.
    pub(super) type_ids: HashMap<ShipId, ShipTypeId>,
}

impl ShipRegistry {
    pub(super) fn new() -> Self {
        Self {
            index: HashMap::new(),
            type_ids: HashMap::new(),
        }
    }

    /// Removes a ship from the identity maps and despawns its ECS entity.
    /// Returns the despawned `Entity`, or `None` if `ship_id` was already gone.
    ///
    /// Player ownership cleanup is deliberately handled by `PlayerState`,
    /// which can apply the active-ship invariant in the same operation.
    pub(super) fn remove(&mut self, ship_id: ShipId, world: &mut SimWorld) -> Option<Entity> {
        let entity = self.index.remove(&ship_id)?;
        world.despawn_ship(entity);
        self.type_ids.remove(&ship_id);
        Some(entity)
    }
}
