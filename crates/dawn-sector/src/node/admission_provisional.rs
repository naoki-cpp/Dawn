//! Current fresh-admission preparation / materialization implementation.
//!
//! # ADR-0049 / #277 migration status
//!
//! The public admission facts remain useful audit output, but the durable
//! reservation and resume-ticket state now goes through the explicit
//! admission/identity repository views in `repositories.rs`.
//!
//! The target contract separates the authorities:
//!
//! - #277 admission/identity repositories own durable pre-materialization
//!   reservation and resume-ticket protocol state;
//! - reserving a `PlayerId` / `ShipId` durably consumes it before `Welcome`, and
//!   abort/expiry never makes that ID reusable;
//! - once a Ship is materialized, Ship/ownership/active-routing/Station world
//!   state is ADR-0049 `RecoveryDelta` + checkpoint authority;
//! - repository finalization after a committed world transition is reconciled
//!   idempotently from a stable admission identity before the affected service is
//!   served; and
//! - public admission events may remain useful facts, but are not the sole exact
//!   recovery reducer or allocator authority.
//!
//! #278 still owns the runtime hook that applies Station projection records
//! after the authoritative RecoveryDelta is durable.

use dawn_core::{
    events::{ClientAdmissionCommitted, ClientAdmissionIdentityReserved},
    fitting::ActivationMode,
    DomainEvent, ItemId, PlayerId, Position, ResumeTicket, ShipId, SlotKind, StationId, Velocity,
};
use dawn_ecs::components::{FittedSlot, FittingComp, InventoryComp, IsNpcComp};
use rand::RngCore;

use super::{HandoffPayload, MissingObserverShip, SimulationNode};

fn generate_resume_ticket() -> ResumeTicket {
    let mut bytes = [0; ResumeTicket::BYTE_LEN];
    rand::thread_rng().fill_bytes(&mut bytes);
    ResumeTicket::from_bytes(bytes)
}

impl SimulationNode {
    /// Durably reserve a fresh identity before producing the admission handoff.
    ///
    /// The repository transaction records the prepared spawn, resume ticket,
    /// consumed identities, and allocator watermarks before the caller can
    /// expose `Welcome`. The public reservation event is an audit fact, not the
    /// allocator authority.
    pub(crate) fn reserve_fresh_admission_identity(
        &mut self,
        spawn_position: Position,
    ) -> (PlayerId, ShipId, ResumeTicket) {
        let resume_ticket = generate_resume_ticket();
        let (player_id, ship_id) = self
            .persistence
            .reserve_fresh_admission_identity(self.node_id, spawn_position, resume_ticket)
            .expect("fresh admission identity reservation");
        self.players.player_id_counter = self.players.player_id_counter.max(player_id.0 + 1);
        self.simulation.id_counter = self.simulation.id_counter.max(ship_id.0.counter() + 1);
        self.emit_event(DomainEvent::ClientAdmissionIdentityReserved(
            ClientAdmissionIdentityReserved {
                player_id,
                ship_id,
                tick: self.simulation.current_tick,
            },
        ));
        let inserted = self.players.pending_fresh_admissions.insert(ship_id);
        debug_assert!(
            inserted,
            "fresh admission ShipId reservation must be unique"
        );
        (player_id, ship_id, resume_ticket)
    }

    pub(crate) fn prepared_fresh_admission(
        &self,
        resume_ticket: ResumeTicket,
    ) -> Option<(PlayerId, ShipId, Position)> {
        self.persistence
            .admissions()
            .prepared_client_admission_by_ticket(resume_ticket)
            .expect("client admission prepared query")
            .map(|prepared| {
                (
                    prepared.player_id,
                    prepared.ship_id,
                    prepared.spawn_position,
                )
            })
    }

    /// True when this Sector can authoritatively handle a resume identity.
    /// Besides materialized Ships, this includes a client-visible fresh
    /// identity whose durable prepared row survived a disconnect or restart.
    pub fn hosts_client_resume_ticket(&self, resume_ticket: ResumeTicket) -> bool {
        self.persistence
            .admissions()
            .prepared_client_admission_by_ticket(resume_ticket)
            .expect("client admission prepared query")
            .is_some()
            || self
                .persistence
                .identities()
                .client_ownership_by_ticket(resume_ticket)
                .expect("client ownership query")
                .is_some_and(|(_, ship_id)| {
                    self.ship_absolute_pos(ship_id).is_some() && !self.is_ship_in_transit(ship_id)
                })
    }

    pub(crate) fn resolve_client_resume_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> Option<(PlayerId, ShipId)> {
        self.persistence
            .identities()
            .client_ownership_by_ticket(resume_ticket)
            .expect("client ownership query")
    }

    pub(crate) fn issue_resume_ticket(&self) -> ResumeTicket {
        generate_resume_ticket()
    }

    pub(crate) fn record_client_resume_ownership(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
    ) {
        self.persistence
            .record_client_ownership(ship_id, player_id, resume_ticket)
            .expect("client ownership upsert");
    }

    pub(crate) fn stage_client_resume_ticket(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        proposed_next_ticket: ResumeTicket,
    ) -> Option<ResumeTicket> {
        self.persistence
            .stage_client_resume_ticket(ship_id, player_id, presented_ticket, proposed_next_ticket)
            .expect("client ownership ticket staging")
    }

    #[cfg(test)]
    pub(crate) fn client_resume_ticket(&self, ship_id: ShipId) -> Option<ResumeTicket> {
        self.persistence
            .identities()
            .client_resume_tickets(ship_id)
            .expect("client ownership tickets query")
            .map(|(current, _pending)| current)
    }

    pub(crate) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> Option<(ResumeTicket, Option<ResumeTicket>)> {
        self.persistence
            .identities()
            .client_resume_tickets(ship_id)
            .expect("client ownership tickets query")
    }

    pub(crate) fn claim_prepared_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        resume_ticket: ResumeTicket,
    ) -> bool {
        if self.players.pending_fresh_admissions.contains(&ship_id)
            || self
                .persistence
                .admissions()
                .prepared_client_admission(ship_id)
                .expect("client admission prepared query")
                .is_none_or(|prepared| {
                    prepared.player_id != player_id || prepared.resume_ticket != resume_ticket
                })
        {
            return false;
        }
        self.players.pending_fresh_admissions.insert(ship_id)
    }

    /// Build a fresh handoff from a temporary in-memory Ship and remove it
    /// before returning. Snapshots therefore never capture an uncommitted Ship.
    pub(crate) fn build_fresh_admission_handoff(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
        aoi_cell_size: f64,
    ) -> Result<HandoffPayload, MissingObserverShip> {
        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        let handoff = self.build_handoff_payload(ship_id, aoi_cell_size);
        self.remove_ship(ship_id);
        handoff
    }

    /// Commit a reserved fresh admission and finalize its repository grant.
    ///
    /// It materializes live state, emits `ClientAdmissionCommitted`, then
    /// finalizes the SQLite grant/identity rows. Retryable finalization repairs
    /// the identity watermark after a restart without replaying the Station
    /// item grant.
    ///
    /// ADR-0049/#272/#277 replaces this as the normative commit contract with a
    /// prepared Sector transition whose `RecoveryDelta` is durable before live
    /// apply, followed by idempotent Admission/Identity repository finalization.
    /// The public event may remain a business fact but is not the sole
    /// recovery authority.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> bool {
        if !self.players.pending_fresh_admissions.contains(&ship_id)
            || self.simulation.ships.index.contains_key(&ship_id)
            || self
                .persistence
                .admissions()
                .prepared_client_admission(ship_id)
                .expect("client admission prepared query")
                .is_none_or(|prepared| {
                    prepared.player_id != player_id
                        || prepared.spawn_position != spawn_position
                        || prepared.resume_ticket != resume_ticket
                })
        {
            return false;
        }

        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return false;
        };
        let fitting = self
            .simulation
            .world
            .get::<FittingComp>(entity)
            .map(|fitting| fitting.to_snapshot())
            .unwrap_or_else(dawn_core::FittingSnapshot::empty);
        let inventory = self
            .simulation
            .world
            .get::<InventoryComp>(entity)
            .map(|inventory| inventory.items.clone())
            .map(|items| {
                items
                    .into_iter()
                    .flat_map(|(item_id, count)| std::iter::repeat_n(item_id, count as usize))
                    .collect()
            })
            .unwrap_or_default();
        let event = ClientAdmissionCommitted {
            player_id,
            ship_id,
            resume_ticket,
            sector_id: self.sector_id,
            initial_position: spawn_position.into(),
            ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
            fitting,
            inventory,
            starter_station_id: StationId(0),
            starter_item_id: ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            starter_item_count: 1,
            tick: self.simulation.current_tick,
        };

        self.emit_event(DomainEvent::ClientAdmissionCommitted(event.clone()));
        self.players.pending_fresh_admissions.remove(&ship_id);
        self.ensure_client_admission_grant(&event);
        true
    }

    /// Release only the in-memory capacity/handshake claim.
    ///
    /// In the current implementation, the pending public output and SQLite data
    /// continue to make the exposed reservation retryable. Under ADR-0049/#277 the stronger
    /// invariant is explicit: terminating the prepared protocol record may
    /// release resources, but its durably consumed IDs are never reusable.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.players.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.simulation.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
    }

    /// True when the requested resume would overwrite a different established
    /// Player/Ship relationship. The explicit IdentityRepository is the
    /// durable fallback; live maps additionally protect active routing.
    pub(crate) fn resume_admission_identity_conflicts(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        self.persistence
            .identities()
            .client_owner(ship_id)
            .expect("client ownership query")
            .is_some_and(|owner| owner != player_id)
            || self
                .players
                .owners
                .get(&ship_id)
                .is_some_and(|owner| *owner != player_id)
            || self
                .players
                .active_ship
                .get(&player_id)
                .is_some_and(|active_ship| *active_ship != ship_id)
    }

    /// Acquire both the Ship and Player sides of the in-flight resume lock.
    pub(crate) fn reserve_resume_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        if !self.simulation.ships.index.contains_key(&ship_id)
            || self
                .players
                .pending_resume_admissions
                .contains_key(&ship_id)
            || self
                .players
                .pending_resume_admissions
                .values()
                .any(|pending_player| *pending_player == player_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            return false;
        }
        self.players
            .pending_resume_admissions
            .insert(ship_id, player_id);
        true
    }

    pub(crate) fn release_resume_admission(&mut self, player_id: PlayerId, ship_id: ShipId) {
        if self.players.pending_resume_admissions.get(&ship_id) == Some(&player_id) {
            self.players.pending_resume_admissions.remove(&ship_id);
        }
    }

    /// Current compare-and-set resume commit path.
    ///
    /// It writes the SQLite ownership/ticket row before exposing the live
    /// ownership maps, which is the current crash-window mitigation. #277/#278
    /// replace this with the explicit identity-repository authority and
    /// reconciliation/serving contract selected by ADR-0049.
    pub(crate) fn commit_reserved_resume_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        presented_ticket: ResumeTicket,
        next_ticket: ResumeTicket,
    ) -> bool {
        if self.players.pending_resume_admissions.get(&ship_id) != Some(&player_id)
            || !self.simulation.ships.index.contains_key(&ship_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        if self
            .persistence
            .identities()
            .client_ownership_by_ticket(presented_ticket)
            .expect("client ownership query")
            != Some((player_id, ship_id))
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        self.persistence
            .record_client_ownership(ship_id, player_id, next_ticket)
            .expect("client ownership upsert");
        let committed = self.resume_player_ship(ship_id, player_id);
        self.release_resume_admission(player_id, ship_id);
        committed
    }

    fn materialize_admission_player_ship(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
    ) {
        self.insert_to_world(ship_id, spawn_position, Velocity::ZERO);
        self.set_spawn_anchor(ship_id, spawn_position);
        self.materialize_ship_stats(
            ship_id,
            crate::ship_types::SHIP_TYPE_MAGPIE,
            dawn_ecs::components::ShipStatsComp::PLAYER,
        );

        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            let _ = self.simulation.world.remove_one::<IsNpcComp>(entity);
        }
        self.players.active_ship.insert(player_id, ship_id);
        self.players.owners.insert(ship_id, player_id);

        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            self.seed_player_inventory(entity);
        }

        for (slot, module_id) in [
            (SlotKind::High, crate::modules::MODULE_RAILGUN_SMALL),
            (SlotKind::Mid, crate::modules::MODULE_AFTERBURNER),
            (SlotKind::Mid, crate::modules::MODULE_FOLD_DISRUPTOR),
        ] {
            self.fit_admission_module_in_memory(ship_id, slot, module_id);
        }
        self.reapply_fitting(ship_id);
    }

    fn fit_admission_module_in_memory(
        &mut self,
        ship_id: ShipId,
        slot: SlotKind,
        module_id: dawn_core::ModuleId,
    ) {
        let Some(def) = self.game_data.module_registry.get(&module_id).cloned() else {
            return;
        };
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return;
        };
        let is_active = matches!(def.activation_mode, ActivationMode::Passive);
        if let Some(mut fitting) = self.simulation.world.get_mut::<FittingComp>(entity) {
            fitting.slot_mut(slot).push(FittedSlot {
                def,
                is_active,
                cycle_remaining: 0,
                target_ship_id: None,
            });
        }
    }
}
