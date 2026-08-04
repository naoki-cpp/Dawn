//! Durable fresh-admission preparation and atomic commit materialization.
//!
//! Begin records an allocation watermark in the event log and the exact spawn
//! input in SQLite before any handshake frame can expose the identity. Commit
//! appends one replay-complete `ClientAdmissionCommitted` event and applies its
//! Station grant, ownership binding, and prepared-row cleanup in one idempotent
//! SQLite transaction.

use dawn_core::{
    events::{ClientAdmissionCommitted, ClientAdmissionIdentityReserved},
    fitting::ActivationMode,
    DomainEvent, ItemId, PlayerId, Position, ResumeTicket, ShipId, SlotKind, StationId, Velocity,
};
use dawn_ecs::components::{FittedSlot, FittingComp, InventoryComp, IsNpcComp};
use dawn_event_store::store::EventStore;
use rand::RngCore;

use super::{
    station_inventory_db::{resume_ticket_expiry, unix_now_secs},
    HandoffPayload, MissingObserverShip, SimulationNode,
};

fn generate_resume_ticket() -> ResumeTicket {
    let mut bytes = [0; ResumeTicket::BYTE_LEN];
    rand::thread_rng().fill_bytes(&mut bytes);
    ResumeTicket::from_bytes(bytes)
}

impl<S: EventStore> SimulationNode<S> {
    /// Reserve identities without materializing a durable Ship. The allocation
    /// watermark is appended first; then SQLite records the spawn input. This
    /// method returns only after both durable writes complete, so a returned
    /// identity is safe for `Welcome` to expose.
    pub(crate) fn reserve_fresh_admission_identity(
        &mut self,
        spawn_position: Position,
    ) -> (PlayerId, ShipId, ResumeTicket) {
        let player_id = self.next_player_id();
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        let resume_ticket = generate_resume_ticket();
        let resume_ticket_expires_at = resume_ticket_expiry(unix_now_secs());
        self.id_counter += 1;
        self.event_store
            .append(DomainEvent::ClientAdmissionIdentityReserved(
                ClientAdmissionIdentityReserved {
                    player_id,
                    ship_id,
                    tick: self.current_tick,
                },
            ));
        self.station_inventory_db
            .reserve_client_admission(
                ship_id,
                player_id,
                spawn_position,
                resume_ticket,
                resume_ticket_expires_at,
            )
            .expect("client admission preparation transaction");
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
            .prepared_client_admission_by_ticket(resume_ticket, unix_now_secs())
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
        self.station_inventory_db
            .prepared_client_admission_by_ticket(resume_ticket, unix_now_secs())
            .expect("prepared client admission query")
            .is_some()
            || self
                .station_inventory_db
                .client_ownership_by_ticket(resume_ticket, unix_now_secs())
                .expect("client ownership query")
                .is_some_and(|(_, ship_id)| {
                    self.ship_absolute_pos(ship_id).is_some() && !self.is_ship_in_transit(ship_id)
                })
    }

    pub(crate) fn resolve_client_resume_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> Option<(PlayerId, ShipId)> {
        self.station_inventory_db
            .client_ownership_by_ticket(resume_ticket, unix_now_secs())
            .expect("client ownership query")
    }

    pub(crate) fn issue_resume_ticket(&self) -> ResumeTicket {
        generate_resume_ticket()
    }

    pub(crate) fn record_client_resume_ownership(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
    ) {
        let resume_ticket_expires_at = resume_ticket_expiry(unix_now_secs());
        self.station_inventory_db
            .record_client_ownership(ship_id, player_id, resume_ticket, resume_ticket_expires_at)
            .expect("client ownership upsert");
    }

    #[cfg(test)]
    pub(crate) fn record_client_resume_ownership_at(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        resume_ticket_expires_at: u64,
    ) {
        self.station_inventory_db
            .record_client_ownership(ship_id, player_id, resume_ticket, resume_ticket_expires_at)
            .expect("client ownership upsert");
    }

    pub(crate) fn stage_client_resume_ticket(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        next_ticket: ResumeTicket,
    ) -> bool {
        let next_ticket_expires_at = resume_ticket_expiry(unix_now_secs());
        self.station_inventory_db
            .stage_client_resume_ticket(
                ship_id,
                player_id,
                presented_ticket,
                next_ticket,
                next_ticket_expires_at,
            )
            .expect("client ownership ticket staging")
    }

    #[cfg(test)]
    pub(crate) fn client_resume_ticket(&self, ship_id: ShipId) -> Option<ResumeTicket> {
        self.station_inventory_db
            .client_resume_tickets(ship_id)
            .expect("client ownership ticket query")
            .map(|(current, _pending)| current.ticket)
    }

    pub(super) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> Option<(
        super::station_inventory_db::StoredResumeTicket,
        Option<super::station_inventory_db::StoredResumeTicket>,
    )> {
        self.station_inventory_db
            .client_resume_tickets(ship_id)
            .expect("client ownership tickets query")
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
                .expect("prepared client admission query")
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

    /// Commit a fresh admission as one replay-complete event plus an idempotent
    /// Station-inventory/identity transaction. The live claim remains held until
    /// the event is durably appended; a crash after append is repaired by
    /// replay/reconcile.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> bool {
        let Some(prepared) = self
            .station_inventory_db
            .prepared_client_admission(ship_id)
            .expect("prepared client admission query")
        else {
            return false;
        };
        if !self.pending_fresh_admissions.contains(&ship_id)
            || self.ships.index.contains_key(&ship_id)
            || prepared.player_id != player_id
            || prepared.spawn_position != spawn_position
            || prepared.resume_ticket != resume_ticket
            || prepared.resume_ticket_expires_at <= unix_now_secs()
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
            resume_ticket_expires_at: prepared.resume_ticket_expires_at,
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

        self.event_store
            .append(DomainEvent::ClientAdmissionCommitted(event.clone()));
        self.pending_fresh_admissions.remove(&ship_id);
        self.ensure_client_admission_grant(&event);
        true
    }

    /// Replay one atomic fresh-admission commit. This method is idempotent for
    /// both ECS state and the SQLite Station/identity ledger.
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

    /// Release only the live capacity claim. The consumed IDs remain in the
    /// event log and the SQLite prepared row remains retryable because a partial
    /// handshake may already have exposed the identity.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
    }

    /// True when the requested resume would overwrite a different established
    /// Player/Ship relationship. SQLite is consulted as the durable fallback
    /// after checkpoint compaction, while the live maps retain session-local
    /// active-Ship constraints.
    pub(crate) fn resume_admission_identity_conflicts(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        self.station_inventory_db
            .client_owner(ship_id)
            .expect("client ownership query")
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

    /// Compare-and-set the exact identity captured at begin. The durable owner
    /// is written before the live ownership maps are exposed, closing the crash
    /// window where a process loss could otherwise forget the binding.
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
            .client_ownership_by_ticket(presented_ticket, unix_now_secs())
            .expect("client ownership query")
            != Some((player_id, ship_id))
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        let promoted = self
            .station_inventory_db
            .promote_client_resume_ticket(ship_id, player_id, next_ticket, unix_now_secs())
            .expect("client ownership ticket promotion");
        if !promoted {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
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
