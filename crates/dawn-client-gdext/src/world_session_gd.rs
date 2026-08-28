use dawn_client_core::{
    ClientFact, ClientState, ClientStateError, PlayerLoadoutMsg, WorldSessionEffect,
    WorldSessionState,
};
use godot::prelude::*;

type Dict = Dictionary<Variant, Variant>;

use crate::id_boundary::{ship_id_from_godot, ship_id_to_godot};
use crate::loadout_gd::PlayerLoadout;
use crate::session_record_gd::{
    BuildableShipType, CapacitorStatus, CelestialBodyRecord, GateRecord, ShipHealth, StationRecord,
};

/// Godot adapter for the pure `dawn-client-core::WorldSessionState` model.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct WorldSession {
    state: WorldSessionState,
}

impl WorldSession {
    pub(crate) fn core_ref(&self) -> &WorldSessionState {
        &self.state
    }

    pub(crate) fn apply_fact(
        &mut self,
        fact: ClientFact,
        loadout: &mut Option<PlayerLoadoutMsg>,
    ) -> Result<WorldSessionEffect, ClientStateError> {
        ClientState::new(&mut self.state, loadout).apply(fact)
    }
}

#[godot_api]
impl WorldSession {
    #[func]
    fn reset(&mut self) {
        self.state.reset();
    }

    #[func]
    fn has_ship(&self, ship_id: i64) -> bool {
        ship_id_from_godot(ship_id).is_some_and(|ship_id| self.state.has_ship(ship_id))
    }

    #[func]
    fn ship_count(&self) -> i64 {
        self.state.ship_count() as i64
    }

    #[func]
    fn player_ship_id(&self) -> i64 {
        self.state
            .player_ship_id()
            .map(ship_id_to_godot)
            .unwrap_or(-1)
    }

    #[func]
    fn player_lock_target(&self) -> i64 {
        self.state
            .player_lock_target()
            .map(ship_id_to_godot)
            .unwrap_or(-1)
    }

    #[func]
    fn advance_client_ticks(&mut self, ticks: i64, mut loadout: Gd<PlayerLoadout>) {
        let mut loadout = loadout.bind_mut();
        self.state.advance_client_ticks(ticks, loadout.core_mut());
    }

    #[func]
    fn is_docked(&self) -> bool {
        self.state.is_docked()
    }

    #[func]
    fn docked_station_id(&self) -> i64 {
        self.state
            .docked_station_id()
            .map(|station_id| i64::from(station_id.0))
            .unwrap_or(-1)
    }

    #[func]
    fn docked_station_name(&self) -> GString {
        self.state.docked_station_name().into()
    }

    #[func]
    fn latest_dock_state_tick(&self) -> i64 {
        self.state.latest_dock_state_tick()
    }

    #[func]
    fn player_ship_type_name(&self) -> GString {
        self.state.player_ship_type_name().into()
    }

    #[func]
    fn player_health(&self) -> Gd<ShipHealth> {
        ShipHealth::wrap(self.state.player_ship_id(), self.state.player_health())
    }

    #[func]
    fn current_tick(&self) -> i64 {
        self.state.current_tick()
    }

    #[func]
    fn current_system_name(&self) -> GString {
        self.state.current_system_name().into()
    }

    #[func]
    fn capacitor_status(&self) -> Gd<CapacitorStatus> {
        CapacitorStatus::wrap(
            self.state.cap_current(),
            self.state.cap_max(),
            self.state.cap_recharge(),
        )
    }

    #[func]
    fn ship_health(&self, ship_id: i64) -> Variant {
        match ship_id_from_godot(ship_id).and_then(|ship_id| {
            self.state
                .ship_hp()
                .get(&ship_id)
                .map(|health| (ship_id, *health))
        }) {
            Some((ship_id, health)) => ShipHealth::wrap(Some(ship_id), health).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn opponent_ship_ids(&self) -> Array<i64> {
        self.state
            .opponent_ship_ids()
            .iter()
            .copied()
            .map(ship_id_to_godot)
            .collect()
    }

    #[func]
    fn gates(&self) -> Array<Gd<GateRecord>> {
        self.state.gates().iter().map(GateRecord::wrap).collect()
    }

    #[func]
    fn stations(&self) -> Array<Gd<StationRecord>> {
        self.state
            .stations()
            .iter()
            .map(StationRecord::wrap)
            .collect()
    }

    #[func]
    fn bodies(&self) -> Array<Gd<CelestialBodyRecord>> {
        self.state
            .bodies()
            .iter()
            .map(CelestialBodyRecord::wrap)
            .collect()
    }

    #[func]
    fn buildable_ship_types(&self) -> Array<Gd<BuildableShipType>> {
        self.state
            .buildable_ship_types()
            .iter()
            .map(BuildableShipType::wrap)
            .collect()
    }

    #[func]
    fn system_names(&self) -> Dict {
        let mut result = Dict::new();
        for (id, name) in self.state.system_names() {
            result.set(i64::from(id.0), name.clone());
        }
        result
    }
}
