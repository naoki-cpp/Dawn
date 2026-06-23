//! Player command dispatch for `SimulationNode`.
//!
//! # Contents
//!
//! ## Movement
//! - `apply_move_command` / `apply_move_command_owned`
//! - `apply_stop_command` / `apply_stop_command_owned`
//! - `owns_ship`
//!
//! ## Private motion helpers (shared with `navigation`)
//! - `is_warping`, `steer_thrust_toward`, `brake_thrust`
//!
//! ## Module commands
//! - `activate_module` / `activate_module_owned`
//! - `deactivate_module` / `deactivate_module_owned`
//! - `set_module_active`
//! - `register_module`, `fit_module`

use dawn_core::{
    DomainEvent, FitModuleCommand, ModuleDefinition, ModuleId, PlayerId, Position, ShipId,
    SlotKind, Velocity,
};
use dawn_ecs::{
    components::{
        ApproachComp, FittedSlot, FittingComp, PositionComp, ShipStatsComp, ThrustComp, WarpComp,
    },
    systems::apply_fitting,
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    // ── Movement commands ─────────────────────────────────────────────────────

    /// Steer `ship_id` toward `target`. Cancels any active warp/approach.
    /// No-op if the ship is unknown, in transit, or in committed warp.
    pub fn apply_move_command(&mut self, ship_id: ShipId, target: Position) {
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return;
        }
        // A committed warp cannot be interrupted; an aligning warp is cancelled
        // (ADR-0022 §7).
        if self.is_warping(entity) {
            return;
        }
        let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
        // Manual thrust overrides any active approach (ADR-0015 §4).
        let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
        let pos = match self.world.inner().get::<&PositionComp>(entity).ok() {
            Some(c) => c.0,
            None => return,
        };
        self.steer_thrust_toward(entity, pos, target);
    }

    /// `apply_move_command` wrapped with an ownership check.
    pub fn apply_move_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        target: Position,
    ) -> bool {
        if !self.owns_ship(player_id, ship_id) {
            return false;
        }
        self.apply_move_command(ship_id, target);
        true
    }

    /// Begin decelerating the ship toward zero velocity using its thrust.
    ///
    /// The movement system applies thrust opposite to velocity each tick until
    /// the ship stops. Cancels any active thrust direction.
    pub fn apply_stop_command(&mut self, ship_id: ShipId) {
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return;
        }
        // A committed warp cannot be interrupted; an aligning warp is cancelled
        // (ADR-0022 §7).
        if self.is_warping(entity) {
            return;
        }
        let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
        // Stopping cancels any active approach (ADR-0015 §4).
        let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
        self.brake_thrust(entity);
    }

    /// Returns `true` if `player_id` owns `ship_id`.
    ///
    /// Used by `_owned` command variants and external player-command dispatch.
    pub fn owns_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.owners.get(&ship_id) == Some(&player_id)
    }

    /// `apply_stop_command` wrapped with ownership check.
    pub fn apply_stop_command_owned(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        if !self.owns_ship(player_id, ship_id) {
            return false;
        }
        self.apply_stop_command(ship_id);
        true
    }

    // ── Private motion helpers (shared with node::navigation) ─────────────────

    /// True if the ship is in the committed warping phase (ADR-0022): its warp
    /// cannot be interrupted by Move/Stop. Aligning or absent warp → false.
    pub(super) fn is_warping(&self, entity: Entity) -> bool {
        self.world
            .inner()
            .get::<&WarpComp>(entity)
            .map(|w| w.is_warping())
            .unwrap_or(false)
    }

    /// Point `entity`'s thrust at `to` from `from` (unit direction, not braking).
    /// Zero thrust if already at the target. Shared by `apply_move_command` and
    /// the Approach System (ADR-0015) so the steering math lives in one place.
    pub(super) fn steer_thrust_toward(&mut self, entity: Entity, from: Position, to: Position) {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let dz = to.z - from.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let dir = if dist > f32::EPSILON {
            Velocity {
                dx: dx / dist,
                dy: dy / dist,
                dz: dz / dist,
            }
        } else {
            Velocity::ZERO
        };
        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.direction = dir;
            t.is_braking = false;
        }
    }

    /// Set `entity`'s thrust to braking (decelerate toward zero velocity).
    /// Shared by `apply_stop_command` and the Approach/Warp Systems.
    pub(super) fn brake_thrust(&mut self, entity: Entity) {
        if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
            t.direction = Velocity::ZERO;
            t.is_braking = true;
        }
    }

    // ── Module commands ───────────────────────────────────────────────────────

    pub fn activate_module(&mut self, cmd: dawn_core::ActivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, true)
    }

    pub fn deactivate_module(&mut self, cmd: dawn_core::DeactivateModuleCommand) -> bool {
        self.set_module_active(cmd.ship_id, cmd.module_id, cmd.slot, false)
    }

    pub fn activate_module_owned(
        &mut self,
        player_id: PlayerId,
        cmd: dawn_core::ActivateModuleCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        self.activate_module(cmd)
    }

    pub fn deactivate_module_owned(
        &mut self,
        player_id: PlayerId,
        cmd: dawn_core::DeactivateModuleCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) {
            return false;
        }
        self.deactivate_module(cmd)
    }

    fn set_module_active(
        &mut self,
        ship_id: ShipId,
        module_id: ModuleId,
        slot: SlotKind,
        active: bool,
    ) -> bool {
        use dawn_core::events::{ModuleActivated, ModuleDeactivated};
        let entity = match self.ships.index.get(&ship_id).copied() {
            Some(e) => e,
            None => return false,
        };

        // Return early if the module is already in the requested state —
        // avoids emitting duplicate ModuleActivated/Deactivated events every tick.
        let already_in_state = self
            .world
            .inner()
            .get::<&FittingComp>(entity)
            .ok()
            .and_then(|f| {
                f.high
                    .iter()
                    .chain(f.mid.iter())
                    .chain(f.low.iter())
                    .chain(f.rig.iter())
                    .find(|s| s.def.id == module_id && s.def.slot == slot)
                    .map(|s| s.is_active == active)
            })
            .unwrap_or(false);
        if already_in_state {
            return true;
        }

        let found = self
            .world
            .inner_mut()
            .get::<&mut FittingComp>(entity)
            .ok()
            .and_then(|mut f| {
                f.find_slot_mut(module_id, slot).map(|s| {
                    s.is_active = active;
                    true
                })
            })
            .unwrap_or(false);

        if !found {
            return false;
        }

        let base = self
            .base_stats
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipStatsComp::NPC);
        apply_fitting(&mut self.world, ship_id, base);

        let event = if active {
            DomainEvent::ModuleActivated(ModuleActivated {
                ship_id,
                module_id,
                slot,
                tick: self.current_tick,
            })
        } else {
            DomainEvent::ModuleDeactivated(ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                tick: self.current_tick,
            })
        };
        self.event_store.append(event);
        true
    }

    // ── Fitting ───────────────────────────────────────────────────────────────

    pub fn register_module(&mut self, def: ModuleDefinition) {
        self.module_registry.insert(def.id, def);
    }

    /// Returns `true` if successful, `false` if the ship or module is unknown.
    pub fn fit_module(&mut self, cmd: FitModuleCommand) -> bool {
        let def = match self.module_registry.get(&cmd.module_id).cloned() {
            Some(d) => d,
            None => return false,
        };
        let entity = match self.ships.index.get(&cmd.ship_id).copied() {
            Some(e) => e,
            None => return false,
        };

        use dawn_core::fitting::ActivationMode;
        let is_npc = self.ships.owners.get(&cmd.ship_id).is_none();
        let is_active = match def.activation_mode {
            ActivationMode::Passive => true,
            ActivationMode::Active => is_npc,
        };
        if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
            fitting.slot_mut(cmd.slot).push(FittedSlot {
                def,
                is_active,
                cycle_remaining: 0,
            });
        } else {
            return false;
        }

        let base = self
            .base_stats
            .get(&cmd.ship_id)
            .copied()
            .unwrap_or(ShipStatsComp::NPC);

        apply_fitting(&mut self.world, cmd.ship_id, base);

        // Append the ShipFitted event
        let snapshot = self
            .world
            .inner()
            .get::<&FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(|_| dawn_core::FittingSnapshot::empty());

        self.event_store
            .append(DomainEvent::ShipFitted(dawn_core::events::ShipFitted {
                ship_id: cmd.ship_id,
                fitting: snapshot,
                tick: self.current_tick,
            }));

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn node_with_modules() -> SimulationNode {
        use crate::{modules, ship_types};
        let mut node = mem_node();
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        node
    }

    #[test]
    fn fitting_same_module_twice_does_not_double_count_stats() {
        use dawn_core::fitting::{ModuleDefinition, ModuleKind, StatDelta};
        use dawn_core::{FitModuleCommand, ModuleId, SlotKind};

        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let railgun = ModuleDefinition {
            id: ModuleId(1),
            name: "Test Railgun".to_string(),
            kind: ModuleKind::Weapon,
            slot: SlotKind::High,
            activation_mode: dawn_core::ActivationMode::Active,
            cap_cost_per_cycle: 60.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta {
                weapon_damage_add: 25.0,
                weapon_range_add: 1000.0,
                ..StatDelta::ZERO
            },
        };
        node.register_module(railgun);

        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });
        let stats_after_first = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(
            stats_after_first.weapon_damage, 25.0,
            "1 module: base(0) + delta(25) = 25"
        );

        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: ModuleId(1),
        });
        let stats_after_second = node.get_ship_stats(ship_id).unwrap();
        assert_eq!(
            stats_after_second.weapon_damage, 50.0,
            "2 modules: base(0) + 2×delta(25) = 50 (not 75 from double-counting)"
        );
    }

    #[test]
    fn player_weapon_deals_damage_to_bot_after_lock_and_activation() {
        use dawn_core::{ActivateModuleCommand, LockOnCommand, ModuleId, SlotKind};

        let mut node = node_with_modules();

        let bot_pos = Position::new(500.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        let player_id = node.next_player_id();
        let player_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        let lock_cmd = LockOnCommand {
            ship_id: player_ship_id,
            target_id: bot_ship_id,
        };

        assert!(
            node.activate_module_owned(
                player_id,
                ActivateModuleCommand {
                    ship_id: player_ship_id,
                    module_id: ModuleId(1),
                    slot: SlotKind::High,
                }
            ),
            "activate_module_owned should return true for player's own ship"
        );

        let mut damage_events = 0;
        for _ in 0..25 {
            let result = node.tick_with_lock_commands(&[lock_cmd.clone()]);
            damage_events += result
                .events
                .iter()
                .filter(|e| matches!(e, DomainEvent::DamageTaken(d) if d.ship_id == bot_ship_id))
                .count();
        }

        assert!(
            damage_events > 0,
            "player should have dealt at least 1 DamageTaken to bot within 25 ticks"
        );
    }
}
