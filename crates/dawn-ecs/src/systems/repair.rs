//! Local repair system for active shield boosters and armor repairers.

use crate::{
    components::{HullComp, ShipIdComp, ShipStatsComp},
    SimWorld,
};
use dawn_core::{
    events::{RepairApplied, RepairLayer},
    fitting::ModuleKind,
    DomainEvent, ShipId, Tick,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepairCycle {
    pub ship_id: ShipId,
    pub module_kind: ModuleKind,
    pub repair_amount: f32,
}

pub struct RepairResult {
    pub events: Vec<DomainEvent>,
}

struct RepairSnap {
    entity: hecs::Entity,
    ship_id: ShipId,
    max_shield: f32,
    max_armor: f32,
    current_shield: f32,
    current_armor: f32,
    current_hull: f32,
    is_destroyed: bool,
}

pub fn run(world: &mut SimWorld, tick: Tick, repair_cycles: &[RepairCycle]) -> RepairResult {
    if repair_cycles.is_empty() {
        return RepairResult { events: Vec::new() };
    }

    let mut snaps: Vec<RepairSnap> = world
        .query::<(&ShipIdComp, &ShipStatsComp, &HullComp)>()
        .iter()
        .map(|(entity, (id, stats, hull))| RepairSnap {
            entity,
            ship_id: id.0,
            max_shield: stats.max_shield,
            max_armor: stats.max_armor,
            current_shield: hull.current_shield,
            current_armor: hull.current_armor,
            current_hull: hull.current_hull,
            is_destroyed: hull.is_destroyed,
        })
        .collect();

    let mut events = Vec::new();
    let mut changed: Vec<usize> = Vec::new();

    for cycle in repair_cycles {
        let Some(index) = snaps.iter().position(|s| s.ship_id == cycle.ship_id) else {
            continue;
        };
        let snap = &mut snaps[index];
        if snap.is_destroyed || cycle.repair_amount <= 0.0 {
            continue;
        }

        let (layer, before, after) = match cycle.module_kind {
            ModuleKind::ShieldBooster => {
                let before = snap.current_shield;
                snap.current_shield =
                    (snap.current_shield + cycle.repair_amount).clamp(0.0, snap.max_shield);
                (RepairLayer::Shield, before, snap.current_shield)
            }
            ModuleKind::ArmorRepairer => {
                let before = snap.current_armor;
                snap.current_armor =
                    (snap.current_armor + cycle.repair_amount).clamp(0.0, snap.max_armor);
                (RepairLayer::Armor, before, snap.current_armor)
            }
            _ => continue,
        };

        let amount = after - before;
        if amount <= 0.0 {
            continue;
        }
        changed.push(index);
        events.push(DomainEvent::RepairApplied(RepairApplied {
            ship_id: snap.ship_id,
            amount,
            layer,
            current_shield: snap.current_shield,
            current_armor: snap.current_armor,
            current_hull: snap.current_hull,
            tick,
        }));
    }

    changed.sort_unstable();
    changed.dedup();
    for index in changed {
        let snap = &snaps[index];
        if let Some(mut hull) = world.get_mut::<HullComp>(snap.entity) {
            hull.current_shield = snap.current_shield;
            hull.current_armor = snap.current_armor;
            hull.current_hull = snap.current_hull;
        }
    }

    RepairResult { events }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{FittedSlot, FittingComp, ShipStatsComp};
    use dawn_core::{
        fitting::{ActivationMode, ModuleDefinition, ModuleId, SlotKind, StatDelta},
        NodeId, Position, SectorId, Velocity,
    };

    fn ship_id() -> ShipId {
        ShipId::new(NodeId(0), 1)
    }

    fn make_world(kind: ModuleKind) -> SimWorld {
        let mut world = SimWorld::new(SectorId(0));
        let entity = world.spawn_ship(ship_id(), Position::ORIGIN, Velocity::ZERO);
        world.set_ship_stats(
            entity,
            ShipStatsComp {
                repair_amount: 40.0,
                ..ShipStatsComp::PLAYER
            },
        );
        if let Some(mut hull) = world.get_mut::<HullComp>(entity) {
            hull.current_shield = 50.0;
            hull.current_armor = 70.0;
        }
        let slot = FittedSlot {
            def: ModuleDefinition {
                id: ModuleId(90),
                name: "Repair".to_string(),
                kind,
                slot: if kind == ModuleKind::ShieldBooster {
                    SlotKind::Mid
                } else {
                    SlotKind::Low
                },
                activation_mode: ActivationMode::Active,
                cap_cost_per_cycle: 10.0,
                cycle_time_ticks: 5,
                stat_delta: StatDelta {
                    repair_amount: 40.0,
                    ..StatDelta::ZERO
                },
            },
            is_active: true,
            cycle_remaining: 0,
        };
        if kind == ModuleKind::ShieldBooster {
            world.get_mut::<FittingComp>(entity).unwrap().mid.push(slot);
        } else {
            world.get_mut::<FittingComp>(entity).unwrap().low.push(slot);
        }
        world
    }

    #[test]
    fn shield_booster_cycle_repairs_current_shield() {
        let mut world = make_world(ModuleKind::ShieldBooster);
        let result = run(
            &mut world,
            Tick(3),
            &[RepairCycle {
                ship_id: ship_id(),
                module_kind: ModuleKind::ShieldBooster,
                repair_amount: 40.0,
            }],
        );
        assert!(matches!(
            result.events.first(),
            Some(DomainEvent::RepairApplied(RepairApplied {
                layer: RepairLayer::Shield,
                amount,
                ..
            })) if (*amount - 40.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn armor_repairer_cycle_repairs_current_armor() {
        let mut world = make_world(ModuleKind::ArmorRepairer);
        let result = run(
            &mut world,
            Tick(3),
            &[RepairCycle {
                ship_id: ship_id(),
                module_kind: ModuleKind::ArmorRepairer,
                repair_amount: 40.0,
            }],
        );
        assert!(matches!(
            result.events.first(),
            Some(DomainEvent::RepairApplied(RepairApplied {
                layer: RepairLayer::Armor,
                amount,
                ..
            })) if (*amount - 40.0).abs() < f32::EPSILON
        ));
    }
}
