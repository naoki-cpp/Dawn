//! Live materialization kernel for Transit handoffs.

use dawn_core::{DomainEvent, Position, TransitHandoffState};
use dawn_ecs::components::{
    CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, ShipStatsComp,
};

use super::super::SimulationNode;

impl SimulationNode {
    /// Complete an incoming Sector Transit from canonical handoff state.
    pub(super) fn import_transit(
        &mut self,
        handoff: &TransitHandoffState,
        from: dawn_core::SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        request_tick: dawn_core::Tick,
    ) {
        for event in self.materialize_incoming_state(
            handoff,
            from,
            entry_pos,
            request_tick,
            self.simulation.current_tick,
        ) {
            self.emit_event(event);
        }
    }

    /// The single mapping from Transit handoff state and its absolute
    /// destination arrival into destination ECS. Anchor selection and
    /// relative-offset derivation stay local to this seam.
    pub(super) fn restore_ship_from_handoff(
        &mut self,
        handoff: &TransitHandoffState,
        entry_pos: dawn_core::AbsolutePosition,
    ) -> (dawn_core::AnchorId, Position) {
        self.insert_to_world(handoff.ship_id, Position::ORIGIN, handoff.velocity);
        let entity = *self
            .simulation
            .ships
            .index
            .get(&handoff.ship_id)
            .expect("inserted Transit handoff must have an ECS entity");
        self.place_entity_at_absolute(entity, entry_pos);
        let anchor = self
            .simulation
            .world
            .ship_anchor(entity)
            .expect("Transit ships always carry AnchorComp");
        let offset = self
            .simulation
            .world
            .get::<PositionComp>(entity)
            .expect("Transit ships always carry PositionComp")
            .0;

        self.simulation
            .ships
            .type_ids
            .insert(handoff.ship_id, handoff.ship_type_id);
        let base = self
            .game_data
            .ship_type_registry
            .get(&handoff.ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);
        self.simulation.base_stats.insert(handoff.ship_id, base);
        self.simulation.world.set_ship_stats(entity, base);
        let fitting = FittingComp::from_snapshot(&handoff.fitting, &self.game_data.module_registry);
        let _ = self.simulation.world.insert_one(entity, fitting);
        self.reapply_fitting(handoff.ship_id);
        if let Some(mut hull) = self.simulation.world.get_mut::<HullComp>(entity) {
            hull.set_hp(
                handoff.current_shield,
                handoff.current_armor,
                handoff.current_hull,
            );
        }
        if let Some(player_id) = handoff.owner_player_id {
            if let Some(resume_ticket) = handoff.resume_ticket {
                self.persistence
                    .record_client_ownership_with_pending(
                        handoff.ship_id,
                        player_id,
                        resume_ticket,
                        handoff.pending_resume_ticket,
                    )
                    .expect("client ownership upsert");
            }
            debug_assert!(self.adopt_player_ship(handoff.ship_id, player_id));
        }
        if let Some(current) = handoff.capacitor {
            let _ = self
                .simulation
                .world
                .insert_one(entity, CapacitorComp { current });
        }
        let _ = self.simulation.world.insert_one(
            entity,
            InventoryComp {
                items: handoff.inventory.clone(),
            },
        );
        (anchor, offset)
    }

    pub(super) fn materialize_incoming_state(
        &mut self,
        handoff: &TransitHandoffState,
        from: dawn_core::SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        request_tick: dawn_core::Tick,
        tick: dawn_core::Tick,
    ) -> [DomainEvent; 2] {
        let (anchor, offset) = self.restore_ship_from_handoff(handoff, entry_pos);
        [
            DomainEvent::AnchorRebased(dawn_core::events::AnchorRebased {
                ship_id: handoff.ship_id,
                anchor,
                offset,
                tick,
            }),
            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                handoff: handoff.clone(),
                from,
                to: self.sector_id,
                request_tick,
                entry_pos,
                tick,
            }),
        ]
    }
}
