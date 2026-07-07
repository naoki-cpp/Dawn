//! Player command dispatch for `SimulationNode`.
//!
//! # Contents
//!
//! ## Unified client dispatch
//! - `apply_client_command` — single entry point for all `ClientCommand`
//!   variants; returns `Some(ClientCommandFollowup)` for commands that need
//!   server-side follow-up (Jump proposal to Raft, fitting JSON resend).
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
    ClientCommand, DomainEvent, FitModuleCommand, JumpCommand, ModuleDefinition, ModuleId,
    PlayerId, Position, ShipId, SlotKind, Velocity,
};

/// What `apply_client_command` hands back to the caller for commands that
/// require server-side context it does not have (Raft handles, session refs).
#[derive(Debug, Clone)]
pub enum ClientCommandFollowup {
    /// Forward to Jump routing (propose Transit to Raft if in range, or let
    /// `apply_jump_with_fallback` start a warp/approach fallback). Carries
    /// the caller's active ship explicitly, since `JumpCommand` itself no
    /// longer does (ADR-0037).
    Jump(ShipId, JumpCommand),
    /// The player's fitting/station-inventory changed (or the attempt was
    /// rejected) — push a refreshed `PlayerLoadout` JSON to this player's
    /// session so the client's UI reflects the authoritative state. Carries
    /// `PlayerId` rather than `ShipId`: some triggers (Disassemble, or
    /// Assemble from a shipless state) leave the caller with no active ship
    /// at all, so a ship_id can't always be resolved back to a player, but a
    /// player_id always identifies the right session
    /// (`docs/architecture/ownership.md` §8).
    RefreshFitting(PlayerId),
}

/// Why an Activate/Deactivate attempt was rejected (ADR-0006/0035).
///
/// Named so a rejection can be logged, tested, and (eventually) surfaced to
/// the client instead of collapsing to a bare `bool` at the call boundary —
/// diagnosing which of these fired used to require temporarily wiring in
/// ad hoc `eprintln!` calls one per branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleActivationRejection {
    /// `player_id` does not own `ship_id` (checked by the `_owned` wrappers).
    NotOwned,
    /// `ship_id` has no entity in this Sector.
    ShipNotFound,
    /// No fitted slot matches `module_id`/`slot`.
    SlotNotFound,
    /// `ModuleKind::requires_target()` and `target.is_some()` disagree —
    /// e.g. a Weapon activated with no target, or a self-only module
    /// activated with one.
    TargetRequirementMismatch,
    /// A target was given but it is not a `Locked` entry in this ship's
    /// `LockComp`.
    TargetNotLocked,
    /// The target is Locked, but beyond the module's effective range
    /// (weapon range+falloff, tackle range, remote-repair range).
    OutOfRange,
}
use dawn_ecs::{
    components::{
        FittedSlot, FittingComp, LockComp, LockState, PositionComp, ThrustComp, WarpComp,
    },
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    // ── Movement commands ─────────────────────────────────────────────────────

    /// Steer `ship_id` toward `target`. Cancels any active warp/approach.
    /// No-op if the ship is unknown, in transit, or in committed warp.
    pub fn apply_move_command(&mut self, ship_id: ShipId, target: Position) {
        if self.is_ship_docked(ship_id) {
            return;
        }
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
        // Manual thrust overrides any active steering mode (Approach ADR-0015
        // §4, Orbit / Keep at Range ADR-0031).
        self.clear_steering_modes(entity);
        let pos = match self.world.inner().get::<&PositionComp>(entity).ok() {
            Some(c) => c.0,
            None => return,
        };
        let target = self
            .dest_in_ship_frame_abs(entity, [target.x as f64, target.y as f64, target.z as f64]);
        self.steer_thrust_toward(entity, pos, target);
    }

    /// `apply_move_command` wrapped with an active-ship check (ADR-0037: only
    /// the caller's active ship can be flown).
    pub fn apply_move_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        target: Position,
    ) -> bool {
        if !self.is_active_ship(player_id, ship_id) {
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
        if self.is_ship_docked(ship_id) {
            return;
        }
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
        // Stopping cancels any active steering mode (Approach ADR-0015 §4,
        // Orbit / Keep at Range ADR-0031).
        self.clear_steering_modes(entity);
        self.brake_thrust(entity);
    }

    /// Returns `true` if `player_id` owns `ship_id`.
    ///
    /// Used by station-management `_owned` variants (Fit/Unfit/Dock/
    /// BuildPackagedShip/DisassembleShip), which operate on any owned ship,
    /// not just the active one (ADR-0037; docs/architecture/ownership.md §7).
    pub fn owns_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.owners.get(&ship_id) == Some(&player_id)
    }

    /// Returns `true` if `ship_id` is `player_id`'s active ship (ADR-0037).
    ///
    /// Implies `owns_ship` (active ⊆ owned) — used by flight/steering/module
    /// `_owned` variants, which may only ever target the one ship a player is
    /// currently flying, never another owned-but-inactive ship.
    pub fn is_active_ship(&self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.ships.active_ship.get(&player_id) == Some(&ship_id)
    }

    /// Apply one `ClientCommand` on behalf of `player_id`.
    ///
    /// Most variants are dispatched to the matching `apply_*_command_owned`
    /// method and return `None`. Two variants need server-side context that
    /// lives outside `dawn-sector` and are handed back as a
    /// `ClientCommandFollowup`:
    ///
    /// - `Jump` → caller proposes Transit to Raft (or starts warp/approach
    ///   fallback via `apply_jump_with_fallback`).
    /// - `Fit` / `Unfit` → caller pushes a refreshed `PlayerLoadout` JSON to
    ///   the session, so a rejected attempt is visibly reverted client-side.
    ///
    /// `lock_commands` accumulates `LockOn` commands for the tick's Lock System
    /// (the caller passes the same vec to `tick_with_lock_commands`).
    pub fn apply_client_command(
        &mut self,
        player_id: PlayerId,
        cmd: ClientCommand,
        lock_commands: &mut Vec<dawn_core::LockOnCommand>,
    ) -> Option<ClientCommandFollowup> {
        // Flight/steering/module/Undock commands never carry a ship_id of
        // their own (ADR-0037) -- they always resolve to the caller's active
        // ship here, so there is no wire-representable way to name a ship the
        // player isn't currently flying. Station inventory-management
        // commands (Fit/Unfit/Dock/BuildPackagedShip/DisassembleShip) still
        // carry an explicit ship_id and check `owns_ship` instead, since they
        // may target any owned docked ship.
        let active_ship = self.ships.active_ship.get(&player_id).copied();
        match cmd {
            ClientCommand::Move(mv) => {
                if let Some(ship_id) = active_ship {
                    self.apply_move_command_owned(player_id, ship_id, mv.target_position);
                }
            }
            ClientCommand::LockOn(lo) => {
                // `lo.ship_id` is not trusted here -- it is resolved fresh
                // from the caller's active ship before the internal
                // `LockOnCommand` (consumed by the Lock System) is built.
                if let Some(ship_id) = active_ship {
                    if !self.is_ship_docked(ship_id) {
                        lock_commands.push(dawn_core::LockOnCommand {
                            ship_id,
                            target_id: lo.target_id,
                        });
                    }
                }
            }
            ClientCommand::Activate(c) => {
                if let Some(ship_id) = active_ship {
                    // The rejection reason (if any) is now a named
                    // ModuleActivationRejection value rather than a discarded
                    // bool -- available to a future caller that wants to log
                    // or surface it. Today's contract stays unchanged: resync
                    // unconditionally, since a rejected activation and an
                    // accepted one both need the client's optimistic HUD
                    // toggle corrected to the authoritative state (ADR-0035).
                    let _ = self.activate_module_owned(player_id, ship_id, c);
                    return Some(ClientCommandFollowup::RefreshFitting(player_id));
                }
            }
            ClientCommand::Deactivate(c) => {
                if let Some(ship_id) = active_ship {
                    let _ = self.deactivate_module_owned(player_id, ship_id, c);
                    return Some(ClientCommandFollowup::RefreshFitting(player_id));
                }
            }
            // Combat is automatic (CombatSystem each tick); AttackCommand is
            // reserved for a future manual-fire mode.
            ClientCommand::Attack(_) => {}
            ClientCommand::Stop(_) => {
                if let Some(ship_id) = active_ship {
                    self.apply_stop_command_owned(player_id, ship_id);
                }
            }
            ClientCommand::Approach(a) => {
                if let Some(ship_id) = active_ship {
                    self.apply_approach_command_owned(player_id, ship_id, a);
                }
            }
            ClientCommand::Warp(w) => {
                if let Some(ship_id) = active_ship {
                    self.apply_warp_command_owned(player_id, ship_id, w);
                }
            }
            ClientCommand::Orbit(o) => {
                if let Some(ship_id) = active_ship {
                    self.apply_orbit_command_owned(player_id, ship_id, o);
                }
            }
            ClientCommand::KeepAtRange(k) => {
                if let Some(ship_id) = active_ship {
                    self.apply_keep_at_range_command_owned(player_id, ship_id, k);
                }
            }
            ClientCommand::Fit(f) => {
                self.fit_module_owned(player_id, f);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::Unfit(u) => {
                self.unfit_module_owned(player_id, u);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::Dock(d) => {
                if let Some(ship_id) = active_ship {
                    self.dock_owned(player_id, ship_id, d);
                    return Some(ClientCommandFollowup::RefreshFitting(player_id));
                }
            }
            ClientCommand::Undock(_) => {
                if let Some(ship_id) = active_ship {
                    self.undock_owned(player_id, ship_id);
                    return Some(ClientCommandFollowup::RefreshFitting(player_id));
                }
            }
            ClientCommand::BuildPackagedShip(b) => {
                self.build_packaged_ship_owned(player_id, b);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::DisassembleShip(d) => {
                self.disassemble_ship_owned(player_id, d);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::Jump(j) => {
                if let Some(ship_id) = active_ship {
                    if self.is_ship_docked(ship_id) {
                        return None;
                    }
                    return Some(ClientCommandFollowup::Jump(ship_id, j));
                }
            }
            ClientCommand::SelectActiveShip(s) => {
                self.select_active_ship_owned(player_id, s);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::Assemble(a) => {
                let _ = self.assemble_ship_owned(player_id, a);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::Disembark(_) => {
                let _ = self.disembark_owned(player_id);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
            ClientCommand::TransferToStation(t) => {
                self.transfer_to_station_owned(player_id, t);
                return Some(ClientCommandFollowup::RefreshFitting(player_id));
            }
        }
        None
    }

    /// `apply_stop_command` wrapped with an active-ship check (ADR-0037).
    pub fn apply_stop_command_owned(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        if !self.is_active_ship(player_id, ship_id) {
            return false;
        }
        self.apply_stop_command(ship_id);
        true
    }

    // ── Private motion helpers (shared with node::navigation) ─────────────────

    /// True if the ship is in the committed warping phase (ADR-0022): its warp
    /// cannot be interrupted by Move/Stop. Aligning or absent warp → false.
    /// Used only by `apply_move_command`, which is the one command allowed to
    /// cancel an *aligning* warp outright (ADR-0022 §7) -- every other
    /// steering command must use `has_active_warp` instead, which also
    /// covers the aligning phase.
    pub(super) fn is_warping(&self, entity: Entity) -> bool {
        self.world
            .inner()
            .get::<&WarpComp>(entity)
            .map(|w| w.is_warping())
            .unwrap_or(false)
    }

    /// True if `entity` has a `WarpComp` in *any* phase, aligning or
    /// committed (ADR-0022/ADR-0031). Warp takes priority over Approach /
    /// Orbit / Keep at Range: a new steering command must not silently race
    /// an in-progress warp, whether or not it has engaged yet. Move/Stop are
    /// the only commands that may interrupt an aligning warp, and they do so
    /// explicitly (`is_warping` + removing `WarpComp`) rather than going
    /// through this check.
    pub(super) fn has_active_warp(&self, entity: Entity) -> bool {
        self.world.inner().get::<&WarpComp>(entity).is_ok()
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

    pub fn activate_module(
        &mut self,
        ship_id: ShipId,
        cmd: dawn_core::ActivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        self.set_module_active(ship_id, cmd.module_id, cmd.slot, true, cmd.target_ship_id)
    }

    pub fn deactivate_module(
        &mut self,
        ship_id: ShipId,
        cmd: dawn_core::DeactivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        self.set_module_active(ship_id, cmd.module_id, cmd.slot, false, None)
    }

    /// `activate_module` wrapped with an active-ship check (ADR-0037).
    pub fn activate_module_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::ActivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        if !self.is_active_ship(player_id, ship_id) {
            return Err(ModuleActivationRejection::NotOwned);
        }
        if self.is_ship_docked(ship_id) {
            return Err(ModuleActivationRejection::ShipNotFound);
        }
        self.activate_module(ship_id, cmd)
    }

    /// `deactivate_module` wrapped with an active-ship check (ADR-0037).
    pub fn deactivate_module_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::DeactivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        if !self.is_active_ship(player_id, ship_id) {
            return Err(ModuleActivationRejection::NotOwned);
        }
        if self.is_ship_docked(ship_id) {
            return Err(ModuleActivationRejection::ShipNotFound);
        }
        self.deactivate_module(ship_id, cmd)
    }

    /// Activate/deactivate a fitted module (ADR-0006, target handling ADR-0035).
    ///
    /// `target` is validated against `ModuleKind::requires_target()`: kinds
    /// that require a target (Weapon, Tackle) are rejected without one, and
    /// kinds that don't are rejected if one is given. When required, `target`
    /// must be a `Locked` entry in this ship's `LockComp` — activation is
    /// rejected against an unlocked or unknown target (Q4/ADR-0035).
    fn set_module_active(
        &mut self,
        ship_id: ShipId,
        module_id: ModuleId,
        slot: SlotKind,
        active: bool,
        target: Option<ShipId>,
    ) -> Result<(), ModuleActivationRejection> {
        use dawn_core::events::{ModuleActivated, ModuleDeactivated};
        use ModuleActivationRejection::*;
        let entity = match self.ships.index.get(&ship_id).copied() {
            Some(e) => e,
            None => return Err(ShipNotFound),
        };

        // Snapshot the slot's current state before mutating anything.
        let current = self
            .world
            .inner()
            .get::<&FittingComp>(entity)
            .ok()
            .and_then(|f| {
                f.iter_slots()
                    .find(|s| s.def.id == module_id && s.def.slot == slot)
                    .map(|s| (s.def.kind, s.is_active, s.target_ship_id))
            });
        let (kind, prev_active, prev_target) = match current {
            Some(c) => c,
            None => return Err(SlotNotFound),
        };

        if active {
            if kind.requires_target() != target.is_some() {
                return Err(TargetRequirementMismatch);
            }
            if let Some(target_id) = target {
                let locked = self
                    .world
                    .inner()
                    .get::<&LockComp>(entity)
                    .ok()
                    .map(|lock| {
                        lock.entries
                            .iter()
                            .any(|e| e.target_id == target_id && e.state == LockState::Locked)
                    })
                    .unwrap_or(false);
                if !locked {
                    return Err(TargetNotLocked);
                }
            }
        }

        // Return early if the module is already in the requested state —
        // avoids emitting duplicate ModuleActivated/Deactivated events every tick.
        if prev_active == active && prev_target == target {
            return Ok(());
        }

        // Tentatively apply, then range-validate against the *post-fit*
        // stats (ADR-0035): a module's own range contribution only shows up
        // in ShipStatsComp after apply_fitting runs, so this can't be
        // checked beforehand. Roll back if it lands out of range — this
        // rejects the activation outright instead of flipping ON then
        // having the Range Gate System flip it back OFF next tick, a
        // same-tick flicker the client can't tell apart from a real cap-out.
        if !self.write_module_slot_state(entity, module_id, slot, active, target) {
            return Err(SlotNotFound);
        }
        self.reapply_fitting(ship_id);

        if active {
            if let Some(target_id) = target {
                if let Some(range) = self.effective_range_for_kind(entity, kind) {
                    if !self.is_target_within_range(ship_id, target_id, range) {
                        self.write_module_slot_state(
                            entity,
                            module_id,
                            slot,
                            prev_active,
                            prev_target,
                        );
                        self.reapply_fitting(ship_id);
                        return Err(OutOfRange);
                    }
                }
            }
        }

        let event = if active {
            DomainEvent::ModuleActivated(ModuleActivated {
                ship_id,
                module_id,
                slot,
                target_ship_id: target,
                tick: self.current_tick,
            })
        } else {
            DomainEvent::ModuleDeactivated(ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                // set_module_active(active=false) is only ever reached via
                // deactivate_module, which is only ever called for a
                // player-issued DeactivateModuleCommand (ADR-0035) — system-
                // forced deactivations (Capacitor/Range Gate) write to
                // FittingComp directly and emit their own events instead.
                forced_reason: None,
                tick: self.current_tick,
            })
        };
        self.event_store.append(event);
        Ok(())
    }

    /// Writes `is_active`/`target_ship_id` onto one fitted slot. Returns
    /// `false` if the slot no longer exists. Does not call `apply_fitting` —
    /// the caller is responsible for that (ADR-0035: `set_module_active`
    /// calls this twice, tentative-apply then possible rollback, and only
    /// wants one `apply_fitting` per attempt).
    fn write_module_slot_state(
        &mut self,
        entity: Entity,
        module_id: ModuleId,
        slot: SlotKind,
        is_active: bool,
        target: Option<ShipId>,
    ) -> bool {
        self.world
            .inner_mut()
            .get::<&mut FittingComp>(entity)
            .ok()
            .and_then(|mut f| {
                f.find_slot_mut(module_id, slot).map(|s| {
                    if is_active {
                        s.is_active = true;
                        s.target_ship_id = target;
                    } else {
                        s.force_off();
                    }
                    true
                })
            })
            .unwrap_or(false)
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
        let is_npc = !self.ships.owners.contains_key(&cmd.ship_id);
        let is_active = match def.activation_mode {
            ActivationMode::Passive => true,
            ActivationMode::Active => is_npc,
        };
        if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity) {
            fitting.slot_mut(cmd.slot).push(FittedSlot {
                def,
                is_active,
                cycle_remaining: 0,
                target_ship_id: None,
            });
        } else {
            return false;
        }

        self.reapply_fitting(cmd.ship_id);
        // Inventory is absent for NPCs and for ships fit_module touches before
        // seeding (ADR-0032) -- this privileged path doesn't consume from it,
        // so emit_ship_fitted just mirrors whatever is currently there for
        // replay fidelity.
        self.emit_ship_fitted(cmd.ship_id, entity);

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::station::StationOperationOutcome;
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

        // Weapon activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        assert!(
            node.activate_module_owned(
                player_id,
                player_ship_id,
                ActivateModuleCommand {
                    module_id: ModuleId(1),
                    slot: SlotKind::High,
                    target_ship_id: Some(bot_ship_id),
                }
            )
            .is_ok(),
            "activate_module_owned should succeed for player's own ship"
        );

        let mut damage_events = 0;
        for _ in 0..25 {
            let result = node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
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

    #[test]
    fn activate_module_owned_reports_the_specific_rejection_reason() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_modules();
        let owner_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(owner_id, Position::ORIGIN);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });

        let intruder_id = node.next_player_id();
        assert_eq!(
            node.activate_module_owned(
                intruder_id,
                ship_id,
                dawn_core::ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            Err(ModuleActivationRejection::NotOwned),
            "a player who isn't flying the ship must be rejected before any fitting lookup"
        );

        assert_eq!(
            node.activate_module_owned(
                owner_id,
                ship_id,
                dawn_core::ActivateModuleCommand {
                    module_id: ModuleId(9999),
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            Err(ModuleActivationRejection::SlotNotFound),
            "activating a module_id that isn't fitted in that slot must name the real reason"
        );
    }

    // ── apply_client_command ─────────────────────────────────────────────────

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    #[test]
    fn owned_move_command_is_applied_and_returns_no_followup() {
        use dawn_core::{ClientCommand, MoveCommand};
        let mut node = mem_node();
        let (player_id, _ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Move(MoveCommand {
                target_position: Position::new(1_000.0, 0.0, 0.0),
            }),
            &mut locks,
        );
        assert!(result.is_none(), "Move must not produce a followup");
    }

    #[test]
    fn move_command_with_no_active_ship_is_silently_ignored_and_returns_no_followup() {
        // ADR-0037: MoveCommand no longer names a ship at all -- there is no
        // longer a wire-representable way to send a Move for a ship the
        // caller doesn't fly. The only remaining rejection path is a player
        // with no active ship at all (e.g. their only ship was destroyed).
        use dawn_core::{ClientCommand, MoveCommand};
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let before = node.total_event_count();
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Move(MoveCommand {
                target_position: Position::new(1_000.0, 0.0, 0.0),
            }),
            &mut locks,
        );
        assert!(
            result.is_none(),
            "Move with no active ship must not produce a followup"
        );
        assert_eq!(
            node.total_event_count(),
            before,
            "Move with no active ship must not append events"
        );
    }

    #[test]
    fn fit_command_returns_refresh_fitting_followup() {
        use crate::modules;
        use dawn_core::{ClientCommand, FitModuleCommand, ModuleId, SlotKind};
        let mut node = mem_node();
        for def in modules::all_modules() {
            node.register_module(def);
        }
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Fit(FitModuleCommand {
                ship_id,
                module_id: ModuleId(1),
                slot: SlotKind::High,
            }),
            &mut locks,
        );
        assert!(
            matches!(result, Some(ClientCommandFollowup::RefreshFitting(id)) if id == player_id),
            "Fit must return RefreshFitting for the caller's player_id"
        );
    }

    #[test]
    fn jump_command_is_handed_back_as_followup() {
        use dawn_core::{ClientCommand, JumpCommand, JumpGateId};
        let mut node = mem_node();
        let (player_id, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let mut locks = Vec::new();
        let result = node.apply_client_command(
            player_id,
            ClientCommand::Jump(JumpCommand {
                gate_id: JumpGateId(0),
            }),
            &mut locks,
        );
        assert!(
            matches!(result, Some(ClientCommandFollowup::Jump(sid, j)) if sid == ship_id && j.gate_id == JumpGateId(0)),
            "Jump must be handed back as a followup"
        );
    }

    #[test]
    fn docked_target_loses_locks_and_cannot_be_damaged_by_active_weapons() {
        use dawn_core::{
            ActivateModuleCommand, DockCommand, LockOnCommand, ModuleId, SlotKind, StationId,
        };

        let mut node = node_with_modules();
        let station = node
            .station(StationId(0))
            .expect("demo station exists")
            .clone();

        let attacker_id = node.next_player_id();
        let attacker_ship_id = node.spawn_player_ship_at_pub(
            attacker_id,
            Position::new(
                station.position.x + 200.0,
                station.position.y,
                station.position.z,
            ),
        );

        let target_player_id = node.next_player_id();
        let target_ship_id = node.spawn_player_ship_at_pub(target_player_id, station.position);

        let lock_cmd = LockOnCommand {
            ship_id: attacker_ship_id,
            target_id: target_ship_id,
        };
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        assert!(
            node.activate_module_owned(
                attacker_id,
                attacker_ship_id,
                ActivateModuleCommand {
                    module_id: ModuleId(1),
                    slot: SlotKind::High,
                    target_ship_id: Some(target_ship_id),
                }
            )
            .is_ok(),
            "attacker should have an active weapon on the target before docking"
        );

        assert!(matches!(
            node.dock_owned(
                target_player_id,
                target_ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let result = node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::LockLost(l) if l.locker_id == attacker_ship_id && l.target_id == target_ship_id)),
            "docking should tear down the attacker's lock on the target"
        );
        assert!(
            !result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::DamageTaken(d) if d.ship_id == target_ship_id)),
            "docked targets must not take combat damage"
        );
    }

    #[test]
    fn move_command_target_is_interpreted_in_the_ships_current_anchor_frame() {
        use dawn_core::AnchorId;

        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let anchor = AnchorId(1);
        let anchor_abs = node.anchor_table().abs(anchor).expect("demo anchor exists");
        let local_pos = Position::new(250.0, 0.0, -100.0);
        node.world.set_ship_anchor(entity, anchor);
        node.world
            .inner_mut()
            .get::<&mut PositionComp>(entity)
            .unwrap()
            .0 = local_pos;

        let target_abs = Position::new(
            (anchor_abs[0] + local_pos.x as f64) as f32,
            (anchor_abs[1] + local_pos.y as f64 + 1_000_000.0) as f32,
            (anchor_abs[2] + local_pos.z as f64) as f32,
        );

        node.apply_move_command(ship_id, target_abs);

        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert!(
            thrust.direction.dy > 0.99,
            "move command should preserve the local +Y intent after an anchor rebase, got {:?}",
            thrust.direction
        );
        assert!(
            thrust.direction.dx.abs() < 0.01 && thrust.direction.dz.abs() < 0.01,
            "move command must not be dominated by the far anchor offset, got {:?}",
            thrust.direction
        );
    }
}
