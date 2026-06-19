use dawn_core::DomainEvent;
use dawn_ecs::systems::{CapacitorSystem, CombatSystem, LockSystem, MovementSystem, apply_fitting};
use dawn_event_store::store::EventStore;

use super::{SimulationNode, TickResult};

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
            if let Some(&base) = self.base_stats.get(ship_id) {
                apply_fitting(&mut self.world, *ship_id, base);
            }
        }

        // 4.5 Tackle System — update TackledComp for active Tackle modules (ADR-0024)
        let tackle_events = self.process_tackle(tick);

        // 5. Lock System — merge human commands with queued bot commands
        let merged_locks: Vec<dawn_core::LockOnCommand> = bot_locks
            .into_iter()
            .chain(lock_commands.iter().cloned())
            .collect();
        let lock = LockSystem(&mut self.world, tick, &merged_locks);

        // 6. Combat System — fire only when the capacitor weapon cycle started this tick
        let combat = CombatSystem(&mut self.world, tick, &cap.weapon_cycles_started);

        // Remove destroyed ships from the ECS and all lookup maps.
        // CLAUDE.md §6: run the Bot System after Combat.
        for ship_id in &combat.destroyed {
            if let Some(entity) = self.ships.index.remove(ship_id) {
                self.world.despawn_ship(entity);
            }
            self.ships.type_ids.remove(ship_id);
            self.base_stats.remove(ship_id);
            if let Some(player_id) = self.ships.owners.remove(ship_id) {
                self.ships.by_player.remove(&player_id);
            }
        }

        // 7. Bot System — bots issue the same commands as human players
        self.process_bots();

        // 8. Append to the EventStore
        let all_events: Vec<DomainEvent> = warp_events.iter()
            .chain(move_events.iter())
            .chain(cap.events.iter())
            .chain(tackle_events.iter())
            .chain(lock.events.iter())
            .chain(combat.events.iter())
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
