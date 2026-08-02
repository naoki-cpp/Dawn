//! Non-durable fresh-admission preview and commit materialization.
//!
//! A fresh handshake needs a real observer-shaped payload before the socket
//! succeeds, but no Ship, event, or Station inventory may survive a process
//! crash before commit. This module reserves identity, materializes the Ship
//! only long enough to build the wire payload, removes it immediately, and
//! performs the durable spawn only after the handshake completes.

use dawn_core::{
    events::ShipSpawned, fitting::ActivationMode, DomainEvent, ItemId, PlayerId, Position, ShipId,
    SlotKind, StationId, Velocity,
};
use dawn_ecs::components::{FittedSlot, FittingComp, IsNpcComp};
use dawn_event_store::store::EventStore;

use super::{HandoffPayload, MissingObserverShip, SimulationNode};

impl<S: EventStore> SimulationNode<S> {
    /// Reserve identities for one in-flight fresh admission without creating
    /// any snapshot- or event-replay-visible Ship state.
    pub(crate) fn reserve_fresh_admission_identity(&mut self) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;
        let inserted = self.pending_fresh_admissions.insert(ship_id);
        debug_assert!(
            inserted,
            "fresh admission ShipId reservation must be unique"
        );
        (player_id, ship_id)
    }

    /// Build a fresh observer handoff from a temporary in-memory Ship.
    ///
    /// The Ship exists only during this call. It is removed before returning,
    /// so periodic snapshots and process crashes cannot persist an uncommitted
    /// admission. The reserved ID remains counted against the population cap.
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

    /// Materialize and persist a previously-reserved fresh admission.
    ///
    /// This is the first point that appends Ship events or writes the starter
    /// Station inventory. A crash before this call therefore leaves no durable
    /// admission residue.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
    ) -> bool {
        if !self.pending_fresh_admissions.remove(&ship_id)
            || self.ships.index.contains_key(&ship_id)
        {
            return false;
        }

        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        self.event_store
            .append(DomainEvent::ShipSpawned(ShipSpawned {
                ship_id,
                sector_id: self.sector_id,
                initial_position: spawn_position.into(),
                ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                tick: self.current_tick,
            }));
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.emit_ship_fitted(ship_id, entity);
        }
        self.credit_station_item(
            player_id,
            StationId(0),
            ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            1,
        );
        true
    }

    /// Release a non-durable fresh-admission reservation after handshake
    /// failure. There is intentionally no ECS, event-log, or SQLite rollback.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
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
