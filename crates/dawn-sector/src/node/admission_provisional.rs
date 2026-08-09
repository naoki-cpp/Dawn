//! Current fresh-admission preparation / materialization implementation.
//!
//! # ADR-0049 / #277 migration status
//!
//! The code below still implements the pre-refactor persistence path: fresh
//! allocation watermarks are appended as public `DomainEvent`s, prepared spawn
//! input and resume-ticket state are stored in the catch-all SQLite adapter, and
//! `ClientAdmissionCommitted` is used by the current replay/reconciliation path.
//! Those mechanics describe the **current migration baseline**, not the final
//! recovery authority selected by ADR-0049.
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
//! Until #277/#278 migrate this code, comments on individual methods explicitly
//! describe current behavior without promoting it to the target persistence
//! contract.

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
    /// Current implementation of a fresh identity reservation.
    ///
    /// Today the allocation watermark is emitted as a public output first and
    /// SQLite then records the exact prepared spawn input. The runtime must
    /// persist the output and the prepared row before exposing the identity;
    /// returning only after both operations is what makes it retryable after
    /// restart.
    ///
    /// Under ADR-0049/#277, the normative invariant is instead that the durable
    /// Admission/Identity repository reservation also durably consumes the
    /// `PlayerId` / `ShipId` before `Welcome`; EventStore replay is migration
    /// baseline rather than the final allocator/recovery authority.
    pub(crate) fn reserve_fresh_admission_identity(
        &mut self,
        spawn_position: Position,
    ) -> (PlayerId, ShipId, ResumeTicket) {
        let player_id = self.next_player_id();
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        let resume_ticket = generate_resume_ticket();
        self.id_counter += 1;
        self.emit_event(DomainEvent::ClientAdmissionIdentityReserved(
            ClientAdmissionIdentityReserved {
                player_id,
                ship_id,
                tick: self.current_tick,
            },
        ));
        self.station_inventory_db.reserve_client_admission(
            ship_id,
            player_id,
            spawn_position,
            resume_ticket,
        );
        let inserted = self.pending_fresh_admissions.insert(ship_id);
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
        self.station_inventory_db
            .prepared_client_admission_by_ticket(resume_ticket)
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
        self.station_inventory_db
            .prepared_client_admission_by_ticket(resume_ticket)
            .is_some()
            || self
                .station_inventory_db
                .client_ownership_by_ticket(resume_ticket)
                .is_some_and(|(_, ship_id)| {
                    self.ship_absolute_pos(ship_id).is_some() && !self.is_ship_in_transit(ship_id)
                })
    }

    pub(crate) fn resolve_client_resume_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> Option<(PlayerId, ShipId)> {
        self.station_inventory_db
            .client_ownership_by_ticket(resume_ticket)
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
        self.station_inventory_db
            .record_client_ownership(ship_id, player_id, resume_ticket);
    }

    pub(crate) fn stage_client_resume_ticket(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        proposed_next_ticket: ResumeTicket,
    ) -> Option<ResumeTicket> {
        self.station_inventory_db.stage_client_resume_ticket(
            ship_id,
            player_id,
            presented_ticket,
            proposed_next_ticket,
        )
    }

    #[cfg(test)]
    pub(crate) fn client_resume_ticket(&self, ship_id: ShipId) -> Option<ResumeTicket> {
        self.station_inventory_db
            .client_resume_tickets(ship_id)
            .map(|(current, _pending)| current)
    }

    pub(crate) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> Option<(ResumeTicket, Option<ResumeTicket>)> {
        self.station_inventory_db.client_resume_tickets(ship_id)
    }

    pub(crate) fn claim_prepared_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        resume_ticket: ResumeTicket,
    ) -> bool {
        if self.pending_fresh_admissions.contains(&ship_id)
            || self
                .station_inventory_db
                .prepared_client_admission(ship_id)
                .is_none_or(|prepared| {
                    prepared.player_id != player_id || prepared.resume_ticket != resume_ticket
                })
        {
            return false;
        }
        self.pending_fresh_admissions.insert(ship_id)
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

    /// Current legacy commit path for a reserved fresh admission.
    ///
    /// It materializes live state, emits `ClientAdmissionCommitted`, then
    /// finalizes the SQLite grant/identity rows. Current
    /// replay/reconciliation repairs several crash cases around that ordering.
    ///
    /// ADR-0049/#272/#277 replaces this as the normative commit contract with a
    /// prepared Sector transition whose `RecoveryDelta` is durable before live
    /// apply, followed by idempotent Admission/Identity repository finalization.
    /// The public event may remain a business fact but is not the sole
    /// replay-complete authority.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> bool {
        if !self.pending_fresh_admissions.contains(&ship_id)
            || self.ships.index.contains_key(&ship_id)
            || self
                .station_inventory_db
                .prepared_client_admission(ship_id)
                .is_none_or(|prepared| {
                    prepared.player_id != player_id
                        || prepared.spawn_position != spawn_position
                        || prepared.resume_ticket != resume_ticket
                })
        {
            return false;
        }

        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        let fitting = self
            .world
            .get::<FittingComp>(entity)
            .map(|fitting| fitting.to_snapshot())
            .unwrap_or_else(dawn_core::FittingSnapshot::empty);
        let inventory = self
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
            tick: self.current_tick,
        };

        self.emit_event(DomainEvent::ClientAdmissionCommitted(event.clone()));
        self.pending_fresh_admissions.remove(&ship_id);
        self.ensure_client_admission_grant(&event);
        true
    }

    /// Replay one fresh-admission public event through the current migration
    /// path. This method is idempotent for current ECS state and the SQLite
    /// Station/identity ledger.
    ///
    /// ADR-0049 does not make this public-event replay the final exact recovery
    /// reducer; future world recovery uses `RecoveryDelta`, with #277 repository
    /// reconciliation for protocol authority.
    #[cfg(test)]
    pub(super) fn replay_client_admission_commit(&mut self, event: &ClientAdmissionCommitted) {
        if !self.ships.index.contains_key(&event.ship_id) {
            self.insert_to_world(event.ship_id, Position::ORIGIN, Velocity::ZERO);
            self.set_spawn_anchor_abs(event.ship_id, event.initial_position);
            self.materialize_ship_stats(
                event.ship_id,
                event.ship_type_id,
                dawn_ecs::components::ShipStatsComp::PLAYER,
            );
            if let Some(&entity) = self.ships.index.get(&event.ship_id) {
                let _ = self.world.remove_one::<IsNpcComp>(entity);
                self.seed_player_inventory(entity);
                let fitting = FittingComp::from_snapshot(&event.fitting, &self.module_registry);
                let _ = self.world.insert_one(entity, fitting);
                let items = event.inventory.iter().copied().fold(
                    std::collections::BTreeMap::new(),
                    |mut items, item_id| {
                        *items.entry(item_id).or_default() += 1;
                        items
                    },
                );
                let _ = self.world.insert_one(entity, InventoryComp { items });
                self.reapply_fitting(event.ship_id);
            }
        }
        self.ships
            .active_ship
            .insert(event.player_id, event.ship_id);
        self.ships.owners.insert(event.ship_id, event.player_id);
        self.player_id_counter = self.player_id_counter.max(event.player_id.0 + 1);
        self.id_counter = self.id_counter.max(event.ship_id.0.counter() + 1);
        self.ensure_client_admission_grant(event);
    }

    /// Release only the in-memory capacity/handshake claim.
    ///
    /// In the current implementation, the pending public output and SQLite data
    /// continue to make the exposed reservation retryable. Under ADR-0049/#277 the stronger
    /// invariant is explicit: terminating the prepared protocol record may
    /// release resources, but its durably consumed IDs are never reusable.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
    }

    /// True when the requested resume would overwrite a different established
    /// Player/Ship relationship. The current path consults the catch-all SQLite
    /// adapter as its durable fallback plus live maps for active routing.
    ///
    /// #277 replaces that adapter with explicit identity authority and #278
    /// gates serving after failover until repository/world reconciliation is
    /// current.
    pub(crate) fn resume_admission_identity_conflicts(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        self.station_inventory_db
            .client_owner(ship_id)
            .is_some_and(|owner| owner != player_id)
            || self
                .ships
                .owners
                .get(&ship_id)
                .is_some_and(|owner| *owner != player_id)
            || self
                .ships
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
        if !self.ships.index.contains_key(&ship_id)
            || self.pending_resume_admissions.contains_key(&ship_id)
            || self
                .pending_resume_admissions
                .values()
                .any(|pending_player| *pending_player == player_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            return false;
        }
        self.pending_resume_admissions.insert(ship_id, player_id);
        true
    }

    pub(crate) fn release_resume_admission(&mut self, player_id: PlayerId, ship_id: ShipId) {
        if self.pending_resume_admissions.get(&ship_id) == Some(&player_id) {
            self.pending_resume_admissions.remove(&ship_id);
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
        if self.pending_resume_admissions.get(&ship_id) != Some(&player_id)
            || !self.ships.index.contains_key(&ship_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        if self
            .station_inventory_db
            .client_ownership_by_ticket(presented_ticket)
            != Some((player_id, ship_id))
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        self.station_inventory_db
            .record_client_ownership(ship_id, player_id, next_ticket);
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

        if let Some(&entity) = self.ships.index.get(&ship_id) {
            let _ = self.world.remove_one::<IsNpcComp>(entity);
        }
        self.ships.active_ship.insert(player_id, ship_id);
        self.ships.owners.insert(ship_id, player_id);

        if let Some(&entity) = self.ships.index.get(&ship_id) {
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
        let Some(def) = self.module_registry.get(&module_id).cloned() else {
            return;
        };
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let is_active = matches!(def.activation_mode, ActivationMode::Passive);
        if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
            fitting.slot_mut(slot).push(FittedSlot {
                def,
                is_active,
                cycle_remaining: 0,
                target_ship_id: None,
            });
        }
    }
}
