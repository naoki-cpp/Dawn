//! The single declaration of a ship's optional ECS components (ADR-0049,
//! issue #312).
//!
//! `SimWorld::capture_optional_components`/`restore_optional_components` and
//! their round-trip test all read the one list declared below. Adding a new
//! optional per-ship component only requires listing it here once; capture,
//! restore, and the test that verifies they agree can no longer drift from
//! each other by construction.

use crate::components::{
    ApproachComp, CapacitorComp, FittingComp, InventoryComp, KeepAtRangeComp, LockComp, OrbitComp,
    TackledComp, ThrustComp, WarpComp, WeaponComp,
};
use crate::world::SimWorld;
use hecs::Entity;

macro_rules! ship_optional_components {
    ($($field:ident : $ty:ty),+ $(,)?) => {
        /// Every optional per-ship ECS component, captured together. `None`
        /// means the ship's entity does not carry that component.
        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct OptionalShipComponents {
            $(pub $field: Option<$ty>,)+
        }

        impl SimWorld {
            /// Capture every optional per-ship component this Sector must be
            /// able to reproduce exactly after a tick rollback or a
            /// checkpoint restore (ADR-0049).
            pub fn capture_optional_components(&self, entity: Entity) -> OptionalShipComponents {
                OptionalShipComponents {
                    $($field: self.get::<$ty>(entity).map(|component| (*component).clone()),)+
                }
            }

            /// Restore every optional per-ship component to exactly the
            /// captured state, removing components the entity currently
            /// carries but `components` does not.
            pub fn restore_optional_components(
                &mut self,
                entity: Entity,
                components: &OptionalShipComponents,
            ) {
                $(
                    match &components.$field {
                        Some(component) => {
                            self.insert_one(entity, component.clone());
                        }
                        None => {
                            self.remove_one::<$ty>(entity);
                        }
                    }
                )+
            }
        }
    };
}

ship_optional_components! {
    capacitor: CapacitorComp,
    weapon: WeaponComp,
    lock: LockComp,
    fitting: FittingComp,
    inventory: InventoryComp,
    approach: ApproachComp,
    orbit: OrbitComp,
    keep_at_range: KeepAtRangeComp,
    warp: WarpComp,
    tackled: TackledComp,
    thrust: ThrustComp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::WarpPhase;
    use dawn_core::navigation::WarpTarget;
    use dawn_core::{AbsolutePosition, JumpGateId, NodeId, Position, SectorId, ShipId, Velocity};

    fn spawn(world: &mut SimWorld) -> Entity {
        world.spawn_ship(ShipId::new(NodeId(0), 1), Position::ORIGIN, Velocity::ZERO)
    }

    #[test]
    fn capture_of_a_freshly_spawned_ship_has_no_transient_steering_or_combat_state() {
        let mut world = SimWorld::new(SectorId(0));
        let entity = spawn(&mut world);

        let captured = world.capture_optional_components(entity);

        // spawn_ship inserts FittingComp/WeaponComp/LockComp/ThrustComp with
        // their empty/zero defaults; the rest genuinely start absent.
        assert!(captured.capacitor.is_none());
        assert!(captured.approach.is_none());
        assert!(captured.orbit.is_none());
        assert!(captured.keep_at_range.is_none());
        assert!(captured.warp.is_none());
        assert!(captured.tackled.is_none());
        assert!(captured.inventory.is_none());
        assert!(captured.fitting.is_some());
        assert!(captured.weapon.is_some());
        assert!(captured.lock.is_some());
        assert!(captured.thrust.is_some());
    }

    #[test]
    fn restore_reproduces_every_present_component() {
        let mut world = SimWorld::new(SectorId(0));
        let entity = spawn(&mut world);
        world.insert_one(entity, CapacitorComp { current: 42.0 });
        world.insert_one(
            entity,
            WarpComp {
                target: WarpTarget::Gate(JumpGateId(1)),
                phase: WarpPhase::Warping,
                auto_jump: false,
                warp_start_abs: AbsolutePosition([0.0, 0.0, 0.0]),
                warp_total: 10,
                warp_elapsed: 3,
                warp_arrival_abs: AbsolutePosition([100.0, 0.0, 0.0]),
                warp_start_vel: Velocity::ZERO,
            },
        );

        let captured = world.capture_optional_components(entity);
        world.remove_one::<CapacitorComp>(entity);
        world.remove_one::<WarpComp>(entity);
        assert!(world.get::<CapacitorComp>(entity).is_none());

        world.restore_optional_components(entity, &captured);

        assert_eq!(world.capture_optional_components(entity), captured);
    }

    #[test]
    fn restore_removes_a_component_absent_from_the_captured_state() {
        let mut world = SimWorld::new(SectorId(0));
        let entity = spawn(&mut world);
        let captured_without_capacitor = world.capture_optional_components(entity);
        world.insert_one(entity, CapacitorComp { current: 42.0 });
        assert!(world.get::<CapacitorComp>(entity).is_some());

        world.restore_optional_components(entity, &captured_without_capacitor);

        assert!(world.get::<CapacitorComp>(entity).is_none());
    }
}
