//! Godot-independent live state for one Dawn client session.
//!
//! [`WorldSessionState`] owns the state derived from InitialState and domain
//! events. It deliberately has no scene-tree or transport dependency; the
//! GDExtension layer adapts its typed values to Godot dictionaries while
//! `main.gd` keeps the Node3D registry and visual side effects.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::PlayerLoadoutMsg;

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct PositionInput {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub z: f64,
}

impl PositionInput {
    pub const fn components(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SystemNameInput {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GateInput {
    #[serde(default)]
    pub gate_id: i64,
    #[serde(default)]
    pub position: PositionInput,
    #[serde(default)]
    pub activation_radius: f64,
    #[serde(default)]
    pub to_system_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct StationInput {
    #[serde(default)]
    pub station_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub position: PositionInput,
    #[serde(default)]
    pub docking_radius: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CelestialBodyInput {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub position: PositionInput,
    #[serde(default = "default_body_radius")]
    pub radius: f64,
    #[serde(default)]
    pub spectral_type: f64,
}

fn default_body_radius() -> f64 {
    1.0
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BuildableShipTypeInput {
    #[serde(default)]
    pub ship_type_id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NavigationInput {
    #[serde(default = "default_system_name")]
    pub system_name: String,
    #[serde(default)]
    pub systems: Vec<SystemNameInput>,
    #[serde(default)]
    pub jump_gates: Vec<GateInput>,
    #[serde(default)]
    pub stations: Vec<StationInput>,
    #[serde(default)]
    pub celestial_bodies: Vec<CelestialBodyInput>,
    #[serde(default)]
    pub buildable_ship_types: Vec<BuildableShipTypeInput>,
}

fn default_system_name() -> String {
    "Unknown".to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ShipInput {
    #[serde(default)]
    pub is_player: bool,
    #[serde(default)]
    pub ship_type_name: String,
    #[serde(default = "default_max_shield")]
    pub max_shield: f64,
    #[serde(default = "default_max_armor")]
    pub max_armor: f64,
    #[serde(default = "default_max_hull")]
    pub max_hull: f64,
    #[serde(default)]
    pub current_shield: Option<f64>,
    #[serde(default)]
    pub current_armor: Option<f64>,
    #[serde(default)]
    pub current_hull: Option<f64>,
    #[serde(default = "default_cap_max")]
    pub cap_max: f64,
    #[serde(default = "default_cap_recharge")]
    pub cap_recharge_per_tick: f64,
}

fn default_max_shield() -> f64 {
    200.0
}

fn default_max_armor() -> f64 {
    150.0
}

fn default_max_hull() -> f64 {
    150.0
}

fn default_cap_max() -> f64 {
    500.0
}

fn default_cap_recharge() -> f64 {
    10.0
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HealthState {
    pub shield: f64,
    pub armor: f64,
    pub hull: f64,
    pub max_shield: f64,
    pub max_armor: f64,
    pub max_hull: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct HealthEventInput {
    #[serde(default)]
    pub ship_id: i64,
    #[serde(default)]
    pub current_shield: f64,
    #[serde(default)]
    pub current_armor: f64,
    #[serde(default)]
    pub current_hull: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShipState {
    pub ship_type_name: String,
    pub is_player: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateRecord {
    pub gate_id: i64,
    pub position: [f64; 3],
    pub activation_radius: f64,
    pub to_system_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StationRecord {
    pub station_id: i64,
    pub name: String,
    pub position: [f64; 3],
    pub docking_radius: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CelestialBodyRecord {
    pub body_id: i64,
    pub kind: String,
    pub name: String,
    pub position: [f64; 3],
    pub radius: f64,
    pub spectral_type: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildableShipTypeRecord {
    pub ship_type_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RegistrationOutcome {
    pub registered: bool,
    pub became_player: bool,
    pub became_opponent: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RemovalOutcome {
    pub removed: bool,
    pub removed_player: bool,
    pub removed_opponent: bool,
    pub cleared_lock: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DestructionOutcome {
    pub destroyed: bool,
    pub destroyed_player: bool,
    pub destroyed_opponent: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HealthEventOutcome {
    pub ship_id: i64,
    pub changed_player: bool,
    pub has_ship: bool,
}

/// The pure client-side state for one connected world session.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSessionState {
    ships: BTreeMap<i64, ShipState>,
    ship_hp: BTreeMap<i64, HealthState>,
    opponent_ship_ids: Vec<i64>,
    gates: Vec<GateRecord>,
    stations: Vec<StationRecord>,
    bodies: Vec<CelestialBodyRecord>,
    buildable_ship_types: Vec<BuildableShipTypeRecord>,
    system_names: BTreeMap<i64, String>,
    player_ship_id: i64,
    player_ship_type_name: String,
    player_health: HealthState,
    player_lock_target: i64,
    current_tick: i64,
    event_count: i64,
    current_system_name: String,
    cap_current: f64,
    cap_max: f64,
    cap_recharge: f64,
    docked_station_id: i64,
    docked_station_name: String,
    latest_dock_state_tick: i64,
}

impl Default for WorldSessionState {
    fn default() -> Self {
        Self {
            ships: BTreeMap::new(),
            ship_hp: BTreeMap::new(),
            opponent_ship_ids: Vec::new(),
            gates: Vec::new(),
            stations: Vec::new(),
            bodies: Vec::new(),
            buildable_ship_types: Vec::new(),
            system_names: BTreeMap::new(),
            player_ship_id: -1,
            player_ship_type_name: String::new(),
            player_health: HealthState {
                max_shield: 500.0,
                max_armor: 300.0,
                max_hull: 200.0,
                ..HealthState::default()
            },
            player_lock_target: -1,
            current_tick: 0,
            event_count: 0,
            current_system_name: "Unknown".to_string(),
            cap_current: -1.0,
            cap_max: 500.0,
            cap_recharge: 10.0,
            docked_station_id: -1,
            docked_station_name: String::new(),
            latest_dock_state_tick: -1,
        }
    }
}

impl WorldSessionState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn has_ship(&self, ship_id: i64) -> bool {
        self.ships.contains_key(&ship_id)
    }

    pub fn ship_count(&self) -> usize {
        self.ships.len()
    }

    pub fn ship_states(&self) -> &BTreeMap<i64, ShipState> {
        &self.ships
    }

    pub fn ship_hp(&self) -> &BTreeMap<i64, HealthState> {
        &self.ship_hp
    }

    pub fn opponent_ship_ids(&self) -> &[i64] {
        &self.opponent_ship_ids
    }

    pub fn gates(&self) -> &[GateRecord] {
        &self.gates
    }

    pub fn stations(&self) -> &[StationRecord] {
        &self.stations
    }

    pub fn bodies(&self) -> &[CelestialBodyRecord] {
        &self.bodies
    }

    pub fn buildable_ship_types(&self) -> &[BuildableShipTypeRecord] {
        &self.buildable_ship_types
    }

    pub fn system_names(&self) -> &BTreeMap<i64, String> {
        &self.system_names
    }

    pub fn player_ship_id(&self) -> i64 {
        self.player_ship_id
    }

    pub fn set_player_ship_id(&mut self, player_ship_id: i64) {
        self.player_ship_id = player_ship_id;
    }

    pub fn player_ship_type_name(&self) -> &str {
        &self.player_ship_type_name
    }

    pub fn player_health(&self) -> HealthState {
        self.player_health
    }

    pub fn player_lock_target(&self) -> i64 {
        self.player_lock_target
    }

    pub fn current_tick(&self) -> i64 {
        self.current_tick
    }

    pub fn event_count(&self) -> i64 {
        self.event_count
    }

    pub fn current_system_name(&self) -> &str {
        &self.current_system_name
    }

    pub fn cap_current(&self) -> f64 {
        self.cap_current
    }

    pub fn cap_max(&self) -> f64 {
        self.cap_max
    }

    pub fn cap_recharge(&self) -> f64 {
        self.cap_recharge
    }

    pub fn docked_station_id(&self) -> i64 {
        self.docked_station_id
    }

    pub fn docked_station_name(&self) -> &str {
        &self.docked_station_name
    }

    pub fn latest_dock_state_tick(&self) -> i64 {
        self.latest_dock_state_tick
    }

    pub fn is_docked(&self) -> bool {
        self.docked_station_id >= 0
    }

    pub fn increment_event_count(&mut self) {
        self.event_count += 1;
    }

    pub fn ingest_navigation(&mut self, input: NavigationInput) {
        self.current_system_name = input.system_name;
        self.system_names = input
            .systems
            .into_iter()
            .map(|system| (system.id, system.name))
            .collect();
        self.gates = input
            .jump_gates
            .into_iter()
            .map(|gate| GateRecord {
                gate_id: gate.gate_id,
                position: gate.position.components(),
                activation_radius: gate.activation_radius,
                to_system_name: gate.to_system_name,
            })
            .collect();
        self.stations = input
            .stations
            .into_iter()
            .map(|station| StationRecord {
                station_id: station.station_id,
                name: station.name,
                position: station.position.components(),
                docking_radius: station.docking_radius,
            })
            .collect();
        self.bodies = input
            .celestial_bodies
            .into_iter()
            .map(|body| CelestialBodyRecord {
                body_id: body.id,
                kind: body.kind,
                name: body.name,
                position: body.position.components(),
                radius: body.radius,
                spectral_type: body.spectral_type,
            })
            .collect();
        self.buildable_ship_types = input
            .buildable_ship_types
            .into_iter()
            .map(|ship| BuildableShipTypeRecord {
                ship_type_id: ship.ship_type_id,
                name: ship.name,
            })
            .collect();
    }

    pub fn register_ship(
        &mut self,
        ship_id: i64,
        input: ShipInput,
        connection_ship_id: i64,
    ) -> RegistrationOutcome {
        if self.ships.contains_key(&ship_id) {
            return RegistrationOutcome::default();
        }

        let health = health_from_ship_input(&input);
        self.ship_hp.insert(ship_id, health);
        self.ships.insert(
            ship_id,
            ShipState {
                ship_type_name: input.ship_type_name.clone(),
                is_player: input.is_player,
            },
        );

        let mut outcome = RegistrationOutcome {
            registered: true,
            ..RegistrationOutcome::default()
        };
        if ship_id == connection_ship_id && self.player_ship_id < 0 {
            self.set_player_ship(ship_id, &input, health);
            outcome.became_player = true;
        } else if input.is_player && !self.opponent_ship_ids.contains(&ship_id) {
            self.opponent_ship_ids.push(ship_id);
            outcome.became_opponent = true;
        }
        outcome
    }

    pub fn remove_ship(&mut self, ship_id: i64, clear_lock: bool) -> RemovalOutcome {
        if self.ships.remove(&ship_id).is_none() {
            return RemovalOutcome::default();
        }
        self.ship_hp.remove(&ship_id);
        let removed_player = ship_id == self.player_ship_id;
        if removed_player {
            self.player_ship_id = -1;
        }
        let removed_opponent = remove_id(&mut self.opponent_ship_ids, ship_id);
        let cleared_lock = clear_lock && ship_id == self.player_lock_target;
        if cleared_lock {
            self.player_lock_target = -1;
        }
        RemovalOutcome {
            removed: true,
            removed_player,
            removed_opponent,
            cleared_lock,
        }
    }

    pub fn destroy_ship(&mut self, ship_id: i64) -> DestructionOutcome {
        if self.ships.remove(&ship_id).is_none() {
            return DestructionOutcome::default();
        }
        self.ship_hp.remove(&ship_id);
        let destroyed_player = ship_id == self.player_ship_id;
        if destroyed_player {
            self.player_ship_id = -1;
            self.player_health.shield = 0.0;
            self.player_health.armor = 0.0;
            self.player_health.hull = 0.0;
            self.player_lock_target = -1;
        }
        let destroyed_opponent = remove_id(&mut self.opponent_ship_ids, ship_id);
        if ship_id == self.player_lock_target {
            self.player_lock_target = -1;
        }
        DestructionOutcome {
            destroyed: true,
            destroyed_player,
            destroyed_opponent,
        }
    }

    pub fn system_changed(&mut self, ship_id: i64, to_system: i64) -> Option<String> {
        if ship_id != self.player_ship_id {
            return None;
        }
        let name = self
            .system_names
            .get(&to_system)
            .cloned()
            .unwrap_or_else(|| format!("System {to_system}"));
        self.current_system_name = name.clone();
        Some(name)
    }

    pub fn apply_hp_event(
        &mut self,
        ship_id: i64,
        shield: f64,
        armor: f64,
        hull: f64,
    ) -> HealthEventOutcome {
        let health = self.ship_hp.entry(ship_id).or_default();
        health.shield = shield;
        health.armor = armor;
        health.hull = hull;
        let changed_player = ship_id == self.player_ship_id;
        if changed_player {
            self.player_health.shield = shield;
            self.player_health.armor = armor;
            self.player_health.hull = hull;
        }
        HealthEventOutcome {
            ship_id,
            changed_player,
            has_ship: self.has_ship(ship_id),
        }
    }

    pub fn apply_target_locked(&mut self, locker_id: i64, target_id: i64) -> bool {
        if locker_id != self.player_ship_id {
            return false;
        }
        self.player_lock_target = target_id;
        true
    }

    pub fn apply_lock_lost(&mut self, locker_id: i64, target_id: i64) -> bool {
        if locker_id != self.player_ship_id {
            return false;
        }
        if target_id == self.player_lock_target {
            self.player_lock_target = -1;
        }
        true
    }

    pub fn advance_tick_from_event(
        &mut self,
        tick: i64,
        loadout: Option<&mut PlayerLoadoutMsg>,
    ) -> i64 {
        if tick <= self.current_tick {
            return 0;
        }
        let ticks_elapsed = tick - self.current_tick;
        self.current_tick = tick;
        self.simulate_cap(ticks_elapsed, loadout);
        ticks_elapsed
    }

    pub fn advance_client_ticks(&mut self, ticks: i64, loadout: Option<&mut PlayerLoadoutMsg>) {
        if ticks <= 0 {
            return;
        }
        self.current_tick += ticks;
        self.simulate_cap(ticks, loadout);
    }

    pub fn apply_dock_event(
        &mut self,
        ship_id: i64,
        station_id: i64,
        station_name: String,
        tick: i64,
    ) -> bool {
        if ship_id != self.player_ship_id {
            return false;
        }
        self.apply_dock_state(station_id, station_name, tick)
    }

    pub fn apply_undock_event(&mut self, ship_id: i64, tick: i64) -> bool {
        if ship_id != self.player_ship_id {
            return false;
        }
        self.apply_dock_state(-1, String::new(), tick)
    }

    pub fn apply_dock_fitting(&mut self, station_id: i64, station_name: String, tick: i64) -> bool {
        self.apply_dock_state(station_id, station_name, tick)
    }

    fn set_player_ship(&mut self, ship_id: i64, input: &ShipInput, health: HealthState) {
        self.player_ship_id = ship_id;
        self.player_ship_type_name = input.ship_type_name.clone();
        self.player_health = health;
        self.cap_max = input.cap_max;
        self.cap_recharge = input.cap_recharge_per_tick;
        self.cap_current = self.cap_max;
    }

    fn apply_dock_state(&mut self, station_id: i64, station_name: String, tick: i64) -> bool {
        if tick < self.latest_dock_state_tick {
            return false;
        }
        self.latest_dock_state_tick = tick;
        self.docked_station_id = station_id;
        self.docked_station_name = station_name;
        true
    }

    fn simulate_cap(&mut self, ticks: i64, loadout: Option<&mut PlayerLoadoutMsg>) {
        if self.cap_current < 0.0 || self.player_ship_id < 0 {
            return;
        }
        let ticks = u32::try_from(ticks).unwrap_or(0);
        self.cap_current = match loadout {
            Some(loadout) => loadout.simulate_capacitor_ticks(
                self.cap_current,
                self.cap_max,
                self.cap_recharge,
                ticks,
            ),
            None => {
                let mut modules = [];
                crate::simulate_modules_capacitor_ticks(
                    &mut modules,
                    self.cap_current,
                    self.cap_max,
                    self.cap_recharge,
                    ticks,
                )
            }
        };
    }
}

fn health_from_ship_input(input: &ShipInput) -> HealthState {
    HealthState {
        shield: input.current_shield.unwrap_or(input.max_shield),
        armor: input.current_armor.unwrap_or(input.max_armor),
        hull: input.current_hull.unwrap_or(input.max_hull),
        max_shield: input.max_shield,
        max_armor: input.max_armor,
        max_hull: input.max_hull,
    }
}

fn remove_id(ids: &mut Vec<i64>, id: i64) -> bool {
    let Some(index) = ids.iter().position(|candidate| *candidate == id) else {
        return false;
    };
    ids.remove(index);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(is_player: bool) -> ShipInput {
        ShipInput {
            is_player,
            ship_type_name: "Magpie".to_string(),
            max_shield: 100.0,
            max_armor: 90.0,
            max_hull: 80.0,
            current_shield: Some(80.0),
            current_armor: Some(70.0),
            current_hull: Some(60.0),
            cap_max: 55.0,
            cap_recharge_per_tick: 3.0,
        }
    }

    #[test]
    fn registering_connection_ship_promotes_it_to_player_state() {
        let mut state = WorldSessionState::default();

        let result = state.register_ship(11, ship(true), 11);

        assert_eq!(
            result,
            RegistrationOutcome {
                registered: true,
                became_player: true,
                became_opponent: false,
            }
        );
        assert_eq!(state.player_ship_id(), 11);
        assert_eq!(state.player_ship_type_name(), "Magpie");
        assert_eq!(state.player_health().shield, 80.0);
        assert_eq!(state.cap_current(), 55.0);
    }

    #[test]
    fn removing_an_aoi_ship_can_preserve_the_lock_target() {
        let mut state = WorldSessionState::default();
        state.register_ship(11, ship(true), 99);
        state.player_ship_id = 99;
        state.register_ship(42, ship(false), 99);
        state.player_lock_target = 42;

        let result = state.remove_ship(42, false);

        assert!(result.removed);
        assert!(!state.has_ship(42));
        assert_eq!(state.player_lock_target(), 42);
    }

    #[test]
    fn stale_dock_context_does_not_overwrite_a_newer_one() {
        let mut state = WorldSessionState::default();
        state.player_ship_id = 7;
        assert!(state.apply_dock_fitting(3, "Forge Station".to_string(), 20));

        assert!(!state.apply_undock_event(7, 19));
        assert_eq!(state.docked_station_id(), 3);
        assert!(state.is_docked());
    }

    #[test]
    fn navigation_ingestion_preserves_absolute_positions() {
        let mut state = WorldSessionState::default();
        state.ingest_navigation(NavigationInput {
            system_name: "Alpha".to_string(),
            systems: vec![SystemNameInput {
                id: 2,
                name: "Beta".to_string(),
            }],
            jump_gates: vec![GateInput {
                gate_id: 7,
                position: PositionInput {
                    x: 149_597_870_710.0,
                    y: 20.0,
                    z: 30.0,
                },
                activation_radius: 1000.0,
                to_system_name: "Beta".to_string(),
            }],
            ..NavigationInput::default()
        });

        assert_eq!(state.current_system_name(), "Alpha");
        assert_eq!(state.system_names().get(&2), Some(&"Beta".to_string()));
        assert_eq!(state.gates()[0].position[0], 149_597_870_710.0);
    }
}
