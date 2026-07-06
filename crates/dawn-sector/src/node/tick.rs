use dawn_core::{DomainEvent, ItemId};
use dawn_ecs::systems::{CapacitorSystem, CombatSystem, LockSystem, MovementSystem, RepairSystem};
use dawn_event_store::store::EventStore;

use super::{SimulationNode, TickResult};

const SCRAP_METAL_PER_SHIP_DESTROYED: u64 = 1;

impl<S: EventStore> SimulationNode<S> {
    /// Execute one simulation tick.
    pub fn tick(&mut self) -> TickResult {
        self.tick_with_lock_commands(&[])
    }

    pub fn tick_with_lock_commands(
        &mut self,
        lock_commands: &[dawn_core::LockOnCommand],
    ) -> TickResult {
        self.current_tick = self.current_tick.next();
        let tick = self.current_tick;

        // 2.5 Approach System — re-aim thrust at approach targets (ADR-0015)
        self.process_approach();

        // 2.55 Orbit System — sweep around the target at a chosen radius (ADR-0031)
        self.process_orbit();

        // 2.56 Keep at Range System — hold at least a chosen range from the target (ADR-0031)
        self.process_keep_at_range();

        // 2.6 Warp System — advance intra-Sector warps (short-range Fold, ADR-0022)
        let warp_events = self.process_warp(tick);

        // 3. Movement System (skips ships in the committed warping phase)
        let move_events = MovementSystem::run(&mut self.world, tick);

        // 4. Capacitor System — recharge and drain for active modules
        let cap = CapacitorSystem(&mut self.world, tick);

        // Drain pending bot lock commands and merge with human-issued ones.
        let bot_locks: Vec<dawn_core::LockOnCommand> =
            std::mem::take(&mut self.pending_bot_lock_commands);
        // Re-apply fitting for any ship whose module was force-deactivated.
        for ship_id in &cap.refitted {
            self.reapply_fitting(*ship_id);
        }

        // 4.5 Tackle System — update TackledComp for active Tackle modules (ADR-0024)
        let tackle_events = self.process_tackle(tick);

        // 5. Lock System — merge human commands with queued bot commands
        let merged_locks: Vec<dawn_core::LockOnCommand> = bot_locks
            .into_iter()
            .chain(lock_commands.iter().cloned())
            .collect();
        let lock = LockSystem(&mut self.world, tick, &merged_locks);

        // Docked ships are not valid space-combat participants: any lock held
        // by a docked ship, or onto a docked ship, is torn down before range
        // gating and combat run for this tick.
        let docked_lock_lost = self.clear_docked_lock_targets(tick);

        // 5.5 Range Gate System — force OFF targeted modules (Weapon/Tackle)
        // whose target has drifted out of effective range (ADR-0035).
        let range_gate_events = self.process_range_gate(tick);

        // 6. Combat System — fire only when the capacitor weapon cycle started this tick.
        // Pass the anchor table so distances resolve across anchors (ADR-0029).
        let combat = CombatSystem(
            &mut self.world,
            tick,
            &cap.weapon_cycles_started,
            self.anchor_table.abs_map(),
        );

        // 6.5 Repair System — apply local repairs after damage for this tick.
        let repair = RepairSystem(&mut self.world, tick, &cap.repair_cycles_started);

        for event in &combat.events {
            let DomainEvent::ShipDestroyed(destroyed) = event else {
                continue;
            };
            let Some(&killer_entity) = self.ships.index.get(&destroyed.killer_id) else {
                continue;
            };
            if let Ok(mut inventory) =
                self.world
                    .inner_mut()
                    .get::<&mut dawn_ecs::components::InventoryComp>(killer_entity)
            {
                inventory.add_item(ItemId::ScrapMetal, SCRAP_METAL_PER_SHIP_DESTROYED);
            }
        }

        // Remove destroyed ships from the ECS and all lookup maps.
        // CLAUDE.md §6: run the Bot System after Combat.
        for ship_id in &combat.destroyed {
            self.remove_ship(*ship_id);
        }

        // 7. Bot System — bots issue the same commands as human players
        self.process_bots();

        // 8. Append to the EventStore
        let all_events: Vec<DomainEvent> = warp_events
            .iter()
            .chain(move_events.iter())
            .chain(cap.events.iter())
            .chain(tackle_events.iter())
            .chain(lock.events.iter())
            .chain(docked_lock_lost.iter())
            .chain(range_gate_events.iter())
            .chain(combat.events.iter())
            .chain(repair.events.iter())
            .cloned()
            .collect();

        let count = all_events.len();
        self.event_store.append_batch(all_events.iter().cloned());

        TickResult {
            tick,
            events_emitted: count,
            events: all_events,
            cap_depletions: cap.refitted.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{modules, ship_types};
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, Tick, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn tick_advances_the_logical_tick_counter_by_one() {
        let mut node = mem_node();
        assert_eq!(node.current_tick(), Tick::ZERO);
        node.tick();
        assert_eq!(node.current_tick(), Tick(1));
        node.tick();
        assert_eq!(node.current_tick(), Tick(2));
    }

    #[test]
    fn npc_ships_at_constant_velocity_produce_no_velocity_changed_events() {
        let mut node = mem_node();
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::new(1.0, 0.0, 0.0),
        );
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(200.0, 100.0, 100.0),
            Velocity::new(0.0, 1.0, 0.0),
        );
        assert_eq!(
            node.tick().events_emitted,
            0,
            "NPC ships at constant velocity do not emit VelocityChanged"
        );
    }

    #[test]
    fn stationary_ships_produce_no_events() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(node.tick().events_emitted, 0);
    }

    #[test]
    fn velocity_changed_events_carry_the_current_tick_value() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::ZERO,
        );
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        node.tick();
        node.tick();
        let last = node.event_store().all_records().last().unwrap();
        assert_eq!(last.event.tick(), Tick(2));
    }

    #[test]
    fn ship_destroyed_immediately_credits_scrap_metal_to_the_killer_inventory() {
        let mut node = mem_node();
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }

        let killer_player = node.next_player_id();
        let killer = node.spawn_player_ship_at_pub(killer_player, Position::ORIGIN);
        let (_bot_player, victim) = node.spawn_bot_ship(Position::new(500.0, 0.0, 0.0));
        let killer_entity = *node.ships.index.get(&killer).unwrap();
        let before = node
            .world
            .inner()
            .get::<&dawn_ecs::components::InventoryComp>(killer_entity)
            .unwrap()
            .item_count(ItemId::ScrapMetal);

        let victim_entity = *node.ships.index.get(&victim).unwrap();
        if let Some(mut hull) = node
            .world
            .get_mut::<dawn_ecs::components::HullComp>(victim_entity)
        {
            hull.set_hp(0.0, 0.0, 1.0);
        }

        let lock_cmd = dawn_core::LockOnCommand {
            ship_id: killer,
            target_id: victim,
        };
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }
        assert!(
            node.activate_module_owned(
                killer_player,
                killer,
                dawn_core::ActivateModuleCommand {
                    module_id: dawn_core::ModuleId(1),
                    slot: dawn_core::SlotKind::High,
                    target_ship_id: Some(victim),
                }
            )
            .is_ok(),
            "weapon activation should succeed once the lock has completed"
        );

        let destroyed = (0..25).any(|_| {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd))
                .events
                .iter()
                .any(
                    |e| matches!(e, DomainEvent::ShipDestroyed(d) if d.ship_id == victim && d.killer_id == killer),
                )
        });
        assert!(
            destroyed,
            "combat should eventually destroy the victim ship"
        );
        let after = node
            .world
            .inner()
            .get::<&dawn_ecs::components::InventoryComp>(killer_entity)
            .unwrap()
            .item_count(ItemId::ScrapMetal);
        assert_eq!(after, before + SCRAP_METAL_PER_SHIP_DESTROYED);
    }
}
