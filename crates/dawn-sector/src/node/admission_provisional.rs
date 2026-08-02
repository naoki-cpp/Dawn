//! Non-durable fresh-admission preview and atomic commit materialization.
//!
//! Begin records only an identity watermark. Commit appends one replay-complete
//! `ClientAdmissionCommitted` event and applies its Station grant through an
//! idempotent SQLite ledger, so every crash boundary converges to the same state.

use dawn_core::{
    events::{ClientAdmissionCommitted, ClientAdmissionIdentityReserved},
    fitting::ActivationMode,
    DomainEvent, ItemId, PlayerId, Position, ShipId, SlotKind, StationId, Velocity,
};
use dawn_ecs::components::{FittedSlot, FittingComp, InventoryComp, IsNpcComp};
use dawn_event_store::store::EventStore;

use super::{HandoffPayload, MissingObserverShip, SimulationNode};

impl<S: EventStore> SimulationNode<S> {
    /// Reserve identities without materializing a durable Ship. The watermark
    /// is appended before any handshake frame can expose either ID.
    pub(crate) fn reserve_fresh_admission_identity(&mut self) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;
        let inserted = self.pending_fresh_admissions.insert(ship_id);
        debug_assert!(
            inserted,
            "fresh admission ShipId reservation must be unique"
        );
        self.event_store
            .append(DomainEvent::ClientAdmissionIdentityReserved(
                ClientAdmissionIdentityReserved {
                    player_id,
                    ship_id,
                    tick: self.current_tick,
                },
            ));
        (player_id, ship_id)
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
    /// Station-inventory grant. The reservation remains held until the event is
    /// durably appended; a crash after append is repaired by replay/reconcile.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
    ) -> bool {
        if !self.pending_fresh_admissions.contains(&ship_id)
            || self.ships.index.contains_key(&ship_id)
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
    /// both ECS state and the Station grant ledger.
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

    /// Release only the live capacity reservation. The consumed IDs remain in
    /// the append-only watermark event and are never reused.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
    }

    /// True when the requested resume would overwrite a different established
    /// Player/Ship relationship. The same exact identity may reconnect and
    /// replace its old runtime session.
    pub(crate) fn resume_admission_identity_conflicts(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        self.ships
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

    /// Compare-and-set the exact identity captured at begin. Ownership may be
    /// absent after restart or already equal during a reconnect, but it may not
    /// have changed to a different Player/Ship while the socket was in flight.
    pub(crate) fn commit_reserved_resume_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        if self.pending_resume_admissions.get(&ship_id) != Some(&player_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
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
