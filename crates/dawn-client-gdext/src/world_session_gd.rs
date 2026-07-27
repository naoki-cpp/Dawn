use dawn_client_core::{
    BuildableShipTypeRecord, CelestialBodyRecord, GateRecord, HealthState, ShipInput,
    StationRecord, WorldSessionState,
};
use godot::prelude::*;
use serde::de::DeserializeOwned;

use crate::json_variant::Dict;
use crate::loadout_gd::PlayerLoadout;

/// Godot adapter for the pure `dawn-client-core::WorldSessionState` model.
///
/// This class owns no Node3D references. `main.gd` keeps the visual ship-node
/// registry and uses the returned outcomes to apply scene-tree side effects.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct WorldSession {
    state: WorldSessionState,
}

#[godot_api]
impl WorldSession {
    #[func]
    fn reset(&mut self) {
        self.state.reset();
    }

    #[func]
    fn has_ship(&self, ship_id: i64) -> bool {
        self.state.has_ship(ship_id)
    }

    #[func]
    fn ship_count(&self) -> i64 {
        self.state.ship_count() as i64
    }

    #[func]
    fn increment_event_count(&mut self) {
        self.state.increment_event_count();
    }

    #[func]
    fn set_player_ship_id(&mut self, ship_id: i64) {
        self.state.set_player_ship_id(ship_id);
    }

    /// Ingests the InitialState navigation portion. Consumes the `Dictionary`
    /// `ServerMessageDecoder` already produced from the decoded
    /// `InitialStateWire` directly (`navigation_gd::navigation_input_from_dict`),
    /// avoiding an extra `JSON.stringify`/`from_str` round trip at the
    /// GDScript-to-Rust boundary just to rebuild a `NavigationInput` that
    /// same-process Dictionary already carries.
    #[func]
    fn ingest_navigation(&mut self, state: Dict) {
        self.state
            .ingest_navigation(crate::navigation_gd::navigation_input_from_dict(&state));
    }

    /// Registers metadata for a ship. The corresponding Node3D is owned by
    /// `main.gd`, not by this state object.
    #[func]
    fn register_ship(&mut self, ship_id: i64, ship_json: GString, connection_ship_id: i64) -> Dict {
        let Some(input) = parse_json::<ShipInput>(&ship_json, "ShipInput") else {
            return Dict::new();
        };
        let outcome = self.state.register_ship(ship_id, input, connection_ship_id);
        let mut result = Dict::new();
        result.set("registered", outcome.registered);
        result.set("became_player", outcome.became_player);
        result.set("became_opponent", outcome.became_opponent);
        result
    }

    #[func]
    fn remove_ship(&mut self, ship_id: i64, clear_lock: bool) -> Dict {
        let outcome = self.state.remove_ship(ship_id, clear_lock);
        let mut result = Dict::new();
        result.set("removed", outcome.removed);
        result.set("removed_player", outcome.removed_player);
        result.set("removed_opponent", outcome.removed_opponent);
        result.set("cleared_lock", outcome.cleared_lock);
        result
    }

    #[func]
    fn destroy_ship(&mut self, ship_id: i64) -> Dict {
        let outcome = self.state.destroy_ship(ship_id);
        let mut result = Dict::new();
        result.set("destroyed", outcome.destroyed);
        result.set("destroyed_player", outcome.destroyed_player);
        result.set("destroyed_opponent", outcome.destroyed_opponent);
        result
    }

    /// Applies a DamageTaken/RepairApplied health update. Takes plain typed
    /// args rather than JSON: the caller (`main.gd`) already has these
    /// values unpacked in the event `Dictionary` `ServerMessageDecoder`
    /// produced from the decoded `EventWire` -- there is no JSON on either
    /// side of this call, so building one here would only add a lossy
    /// stringify/parse round trip (JSON can't represent `NaN`/`Infinity`,
    /// which `f32`/`f64` can).
    #[func]
    fn apply_health_event(&mut self, ship_id: i64, shield: f64, armor: f64, hull: f64) -> Dict {
        let outcome = self.state.apply_hp_event(ship_id, shield, armor, hull);
        let mut result = Dict::new();
        result.set("ship_id", outcome.ship_id);
        result.set("changed_player", outcome.changed_player);
        result.set("has_ship", outcome.has_ship);
        result
    }

    #[func]
    fn apply_target_locked(&mut self, locker_id: i64, target_id: i64) -> bool {
        self.state.apply_target_locked(locker_id, target_id)
    }

    #[func]
    fn apply_lock_lost(&mut self, locker_id: i64, target_id: i64) -> bool {
        self.state.apply_lock_lost(locker_id, target_id)
    }

    #[func]
    fn system_changed(&mut self, ship_id: i64, to_system: i64) -> Dict {
        let mut result = Dict::new();
        let Some(name) = self.state.system_changed(ship_id, to_system) else {
            result.set("changed_player", false);
            return result;
        };
        result.set("changed_player", true);
        result.set("system_name", name);
        result
    }

    #[func]
    fn advance_tick_from_event(&mut self, tick: i64, mut loadout: Gd<PlayerLoadout>) -> i64 {
        let mut loadout = loadout.bind_mut();
        self.state.advance_tick_from_event(tick, loadout.core_mut())
    }

    #[func]
    fn advance_client_ticks(&mut self, ticks: i64, mut loadout: Gd<PlayerLoadout>) {
        let mut loadout = loadout.bind_mut();
        self.state.advance_client_ticks(ticks, loadout.core_mut());
    }

    #[func]
    fn apply_dock_event(
        &mut self,
        ship_id: i64,
        station_id: i64,
        station_name: GString,
        tick: i64,
    ) -> bool {
        self.state
            .apply_dock_event(ship_id, station_id, station_name.to_string(), tick)
    }

    #[func]
    fn apply_undock_event(&mut self, ship_id: i64, tick: i64) -> bool {
        self.state.apply_undock_event(ship_id, tick)
    }

    #[func]
    fn apply_dock_fitting(&mut self, station_id: i64, station_name: GString, tick: i64) -> bool {
        self.state
            .apply_dock_fitting(station_id, station_name.to_string(), tick)
    }

    #[func]
    fn is_docked(&self) -> bool {
        self.state.is_docked()
    }

    #[func]
    fn dock_status(&self) -> Dict {
        let mut result = Dict::new();
        result.set("docked_station_id", self.state.docked_station_id());
        result.set(
            "docked_station_name",
            self.state.docked_station_name().to_string(),
        );
        result.set("is_docked", self.state.is_docked());
        result.set(
            "latest_dock_state_tick",
            self.state.latest_dock_state_tick(),
        );
        result
    }

    /// Returns the complete state projection consumed by `main.gd` and HUD.
    /// Keeping this as one snapshot prevents GDScript from maintaining aliases
    /// into mutable state owned by the Rust object.
    #[func]
    fn snapshot(&self) -> Dict {
        let mut result = Dict::new();
        result.set("ship_hp", &self.ship_hp());
        result.set("opponent_ship_ids", &self.opponent_ship_ids());
        result.set("gates", &self.gates());
        result.set("stations", &self.stations());
        result.set("bodies", &self.bodies());
        result.set("buildable_ship_types", &self.buildable_ship_types());
        result.set("system_names", &self.system_names());
        result.set("player_ship_id", self.state.player_ship_id());
        result.set(
            "player_ship_type_name",
            self.state.player_ship_type_name().to_string(),
        );
        let player_health = self.state.player_health();
        result.set("player_shield", player_health.shield);
        result.set("player_armor", player_health.armor);
        result.set("player_hull", player_health.hull);
        result.set("player_max_shield", player_health.max_shield);
        result.set("player_max_armor", player_health.max_armor);
        result.set("player_max_hull", player_health.max_hull);
        result.set("player_lock_target", self.state.player_lock_target());
        result.set("current_tick", self.state.current_tick());
        result.set("event_count", self.state.event_count());
        result.set(
            "current_system_name",
            self.state.current_system_name().to_string(),
        );
        result.set("cap_current", self.state.cap_current());
        result.set("cap_max", self.state.cap_max());
        result.set("cap_recharge", self.state.cap_recharge());
        result
    }

    fn ship_hp(&self) -> Dict {
        let mut result = Dict::new();
        for (ship_id, health) in self.state.ship_hp() {
            let health = health_dict(*ship_id, *health);
            result.set(*ship_id, &health);
        }
        result
    }

    fn opponent_ship_ids(&self) -> Array<i64> {
        self.state.opponent_ship_ids().iter().copied().collect()
    }

    fn gates(&self) -> Array<Dict> {
        self.state.gates().iter().map(gate_dict).collect()
    }

    fn stations(&self) -> Array<Dict> {
        self.state.stations().iter().map(station_dict).collect()
    }

    fn bodies(&self) -> Array<Dict> {
        self.state.bodies().iter().map(body_dict).collect()
    }

    fn buildable_ship_types(&self) -> Array<Dict> {
        self.state
            .buildable_ship_types()
            .iter()
            .map(buildable_ship_type_dict)
            .collect()
    }

    fn system_names(&self) -> Dict {
        let mut result = Dict::new();
        for (id, name) in self.state.system_names() {
            result.set(*id, name.clone());
        }
        result
    }
}

fn parse_json<T: DeserializeOwned>(json: &GString, context: &str) -> Option<T> {
    match serde_json::from_str(&json.to_string()) {
        Ok(value) => Some(value),
        Err(err) => {
            godot_error!("WorldSession.{context}: {err}");
            None
        }
    }
}

fn health_dict(ship_id: i64, health: HealthState) -> Dict {
    let mut result = Dict::new();
    result.set("ship_id", ship_id);
    result.set("shield", health.shield);
    result.set("armor", health.armor);
    result.set("hull", health.hull);
    result.set("max_shield", health.max_shield);
    result.set("max_armor", health.max_armor);
    result.set("max_hull", health.max_hull);
    result
}

fn gate_dict(gate: &GateRecord) -> Dict {
    let mut result = Dict::new();
    result.set("gate_id", gate.gate_id);
    let position = PackedFloat64Array::from(gate.position);
    result.set("position", &position);
    result.set("activation_radius", gate.activation_radius);
    result.set("to_system_name", gate.to_system_name.clone());
    result
}

fn station_dict(station: &StationRecord) -> Dict {
    let mut result = Dict::new();
    result.set("station_id", station.station_id);
    result.set("name", station.name.clone());
    let position = PackedFloat64Array::from(station.position);
    result.set("position", &position);
    result.set("docking_radius", station.docking_radius);
    result
}

fn body_dict(body: &CelestialBodyRecord) -> Dict {
    let mut result = Dict::new();
    result.set("body_id", body.body_id);
    result.set("kind", body.kind.clone());
    result.set("name", body.name.clone());
    let position = PackedFloat64Array::from(body.position);
    result.set("position", &position);
    result.set("radius", body.radius);
    result.set("spectral_type", body.spectral_type);
    result
}

fn buildable_ship_type_dict(ship: &BuildableShipTypeRecord) -> Dict {
    let mut result = Dict::new();
    result.set("ship_type_id", ship.ship_type_id);
    result.set("name", ship.name.clone());
    result
}
