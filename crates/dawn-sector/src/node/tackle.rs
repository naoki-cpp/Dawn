use std::collections::HashMap;

use dawn_core::{DomainEvent, ShipId, Tick};
use dawn_core::events::{TackleApplied, TackleReleased};
use dawn_core::fitting::ModuleKind;
use dawn_ecs::components::{FittingComp, LockComp, PositionComp, ShipStatsComp, TackledComp};
use dawn_ecs::Entity;
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Tackle System — Step 4.5 (after Capacitor, before Lock). ADR-0024.
    ///
    /// Computes the desired tackle state from active Fold Disruptors + locked
    /// targets in range, diffs against the current `TackledComp` state, emits
    /// `TackleApplied`/`TackleReleased` events for changes, and updates the ECS.
    pub fn process_tackle(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // Collect ships with at least one active Tackle module.
        let tacklers: Vec<(ShipId, f32, Vec<ShipId>, dawn_core::Position)> = self.ships.index.iter()
            .filter_map(|(&ship_id, &entity)| {
                let stats = self.world.inner().get::<&ShipStatsComp>(entity).ok()?;
                if stats.tackle_range <= 0.0 { return None; }
                let fitting = self.world.inner().get::<&FittingComp>(entity).ok()?;
                if !fitting.has_active_module_of_kind(ModuleKind::Tackle) { return None; }
                let lock = self.world.inner().get::<&LockComp>(entity).ok()?;
                let locked: Vec<ShipId> = lock.locked_targets().collect();
                if locked.is_empty() { return None; }
                let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
                Some((ship_id, stats.tackle_range, locked, pos))
            })
            .collect();

        // desired[target] = Vec of tacklers currently in range and holding a lock.
        let mut desired: HashMap<ShipId, Vec<ShipId>> = HashMap::new();
        for (tackler_id, range, locked, tackler_pos) in &tacklers {
            for &target_id in locked {
                if let Some(&te) = self.ships.index.get(&target_id) {
                    if let Ok(tp) = self.world.inner().get::<&PositionComp>(te) {
                        if tackler_pos.distance(tp.0) <= *range {
                            desired.entry(target_id).or_default().push(*tackler_id);
                        }
                    }
                }
            }
        }

        // Snapshot current tackle state — single ECS scan.
        let current: Vec<(ShipId, Entity, Vec<ShipId>)> = self.ships.index.iter()
            .filter_map(|(&sid, &entity)| {
                let t = self.world.inner().get::<&TackledComp>(entity).ok()?;
                Some((sid, entity, t.tacklers.clone()))
            })
            .collect();
        let current_ids: std::collections::HashSet<ShipId> =
            current.iter().map(|(s, _, _)| *s).collect();

        let mut events: Vec<DomainEvent> = Vec::new();

        // Update existing TackledComps: emit diffs, then write new list.
        for (target_id, entity, old_tacklers) in current {
            let new_tacklers = desired.get(&target_id).cloned().unwrap_or_default();

            for &tid in &old_tacklers {
                if !new_tacklers.contains(&tid) {
                    events.push(DomainEvent::TackleReleased(TackleReleased { ship_id: target_id, by: tid, tick }));
                }
            }
            for &tid in &new_tacklers {
                if !old_tacklers.contains(&tid) {
                    events.push(DomainEvent::TackleApplied(TackleApplied { ship_id: target_id, by: tid, tick }));
                }
            }

            if new_tacklers.is_empty() {
                let _ = self.world.inner_mut().remove_one::<TackledComp>(entity);
            } else {
                if let Ok(mut tackled) = self.world.inner_mut().get::<&mut TackledComp>(entity) {
                    tackled.tacklers = new_tacklers;
                }
            }
        }

        // Insert TackledComp for ships newly entering tackled state.
        for (&target_id, new_tacklers) in &desired {
            if current_ids.contains(&target_id) { continue; }
            for &tid in new_tacklers {
                events.push(DomainEvent::TackleApplied(TackleApplied { ship_id: target_id, by: tid, tick }));
            }
            if let Some(&entity) = self.ships.index.get(&target_id) {
                let _ = self.world.inner_mut().insert_one(entity, TackledComp { tacklers: new_tacklers.clone() });
            }
        }

        events
    }
}
