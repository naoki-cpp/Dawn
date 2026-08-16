//! Module activation and registry command policy for [`SimulationNode`].
//!
//! This module owns activation/deactivation policy. The exhaustive external
//! request match lives in `node::commands` and calls these family-local methods
//! directly, so this module does not maintain a parallel dispatch catalog.

use dawn_core::{DomainEvent, FitModuleCommand, ModuleId, PlayerId, ShipId, SlotKind};
use dawn_ecs::{
    components::{FittedSlot, FittingComp, LockComp, LockState},
    Entity,
};

use super::SimulationNode;

/// Why an Activate/Deactivate attempt was rejected (ADR-0006/0035).
///
/// Named so a rejection can be logged, tested, and (eventually) surfaced to
/// the client instead of collapsing to a bare `bool` at the call boundary —
/// diagnosing which of these fired used to require temporarily wiring in
/// ad hoc `eprintln!` calls one per branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleActivationRejection {
    /// `player_id` does not own `ship_id`, or it isn't their active ship
    /// (checked by the `_owned` wrappers via `resolve_flight_command`).
    NotOwned,
    /// `ship_id` has no entity in this Sector.
    ShipNotFound,
    /// `ship_id` is docked; module activation requires being undocked.
    ShipDocked,
    /// `ship_id` is frozen while a Sector Transit handoff is pending.
    ShipInTransit,
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

/// Maps a `resolve_flight_command` rejection (`ship_command.rs`) onto the
/// `ModuleActivationRejection` reported by `activate_module_owned`/
/// `deactivate_module_owned`. `MustBeDocked` is fitting-only and can never
/// be produced by `resolve_flight_command`.
fn module_activation_rejection_from_flight(
    rejection: super::ship_command::ShipCommandRejection,
) -> ModuleActivationRejection {
    use super::ship_command::ShipCommandRejection;
    match rejection {
        ShipCommandRejection::NotOwned | ShipCommandRejection::NotActiveShip => {
            ModuleActivationRejection::NotOwned
        }
        ShipCommandRejection::ShipNotFound => ModuleActivationRejection::ShipNotFound,
        ShipCommandRejection::MustBeUndocked => ModuleActivationRejection::ShipDocked,
        ShipCommandRejection::MustBeDocked => {
            unreachable!("resolve_flight_command never returns MustBeDocked (fitting-command only)")
        }
    }
}

impl SimulationNode {
    // ── Module commands ───────────────────────────────────────────────────────

    pub(crate) fn activate_module(
        &mut self,
        ship_id: ShipId,
        cmd: dawn_core::ActivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        self.set_module_active(ship_id, cmd.module_id, cmd.slot, true, cmd.target_ship_id)
    }

    pub(crate) fn deactivate_module(
        &mut self,
        ship_id: ShipId,
        cmd: dawn_core::DeactivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        self.set_module_active(ship_id, cmd.module_id, cmd.slot, false, None)
    }

    /// `activate_module` wrapped with the shared flight-command seam
    /// (active-ship + undocked, ADR-0037; `ship_command.rs`).
    pub(crate) fn activate_module_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::ActivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        if let Err(rejection) = self.resolve_flight_command(player_id, ship_id) {
            return Err(module_activation_rejection_from_flight(rejection));
        }
        self.activate_module(ship_id, cmd)
    }

    /// `deactivate_module` wrapped with the shared flight-command seam
    /// (active-ship + undocked, ADR-0037; `ship_command.rs`).
    pub(crate) fn deactivate_module_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::DeactivateModuleCommand,
    ) -> Result<(), ModuleActivationRejection> {
        if let Err(rejection) = self.resolve_flight_command(player_id, ship_id) {
            return Err(module_activation_rejection_from_flight(rejection));
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
        let entity = match self.simulation.ships.index.get(&ship_id).copied() {
            Some(e) => e,
            None => return Err(ShipNotFound),
        };
        if self.simulation.world.transit_state(entity).is_in_transit() {
            return Err(ShipInTransit);
        }

        // Snapshot the slot's current state before mutating anything.

        let current = self
            .simulation
            .world
            .get::<FittingComp>(entity)
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
                    .simulation
                    .world
                    .get::<LockComp>(entity)
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
                tick: self.simulation.current_tick,
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
                tick: self.simulation.current_tick,
            })
        };
        self.emit_event(event);
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
        self.simulation
            .world
            .get_mut::<FittingComp>(entity)
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

    /// Returns `true` if successful, `false` if the ship or module is unknown.
    pub fn fit_module(&mut self, cmd: FitModuleCommand) -> bool {
        let def = match self.game_data.module_registry.get(&cmd.module_id).cloned() {
            Some(d) => d,
            None => return false,
        };
        let entity = match self.simulation.ships.index.get(&cmd.ship_id).copied() {
            Some(e) => e,
            None => return false,
        };

        use dawn_core::fitting::ActivationMode;
        let is_npc = !self.players.owners.contains_key(&cmd.ship_id);
        let is_active = match def.activation_mode {
            ActivationMode::Passive => true,
            ActivationMode::Active => is_npc,
        };
        if let Some(mut fitting) = self.simulation.world.get_mut::<FittingComp>(entity) {
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
