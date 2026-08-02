//! Godot-independent live state for one Dawn client session.
//!
//! [`WorldSessionState`] owns the state derived from InitialState and domain
//! events. It deliberately has no scene-tree or transport dependency; the
//! GDExtension layer converts wire values into [`WorldSessionUpdate`] and
//! exposes typed presentation records while `main.gd` keeps the Node3D
//! registry and visual side effects.

use std::collections::BTreeMap;

use crate::PlayerLoadoutMsg;

/// Typed navigation input built by the external adapter. The client-core
/// crate remains wire/JSON/Godot-agnostic per ADR-0039.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PositionInput {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl PositionInput {
    pub const fn components(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemNameInput {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GateInput {
    pub gate_id: i64,
    pub position: PositionInput,
    pub activation_radius: f64,
    pub to_system_name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StationInput {
    pub station_id: i64,
    pub name: String,
    pub position: PositionInput,
    pub docking_radius: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CelestialBodyInput {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub position: PositionInput,
    /// Defaults to `1.0` (not `0.0`) when built from a Dict/JSON that omits
    /// it -- a radius of 0 would make the body's docking/collision math
    /// degenerate, matching the old `#[serde(default = "default_body_radius")]`.
    pub radius: f64,
    pub spectral_type: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildableShipTypeInput {
    pub ship_type_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationInput {
    pub system_name: String,
    pub systems: Vec<SystemNameInput>,
    pub jump_gates: Vec<GateInput>,
    pub stations: Vec<StationInput>,
    pub celestial_bodies: Vec<CelestialBodyInput>,
    pub buildable_ship_types: Vec<BuildableShipTypeInput>,
}

impl Default for NavigationInput {
    fn default() -> Self {
        Self {
            system_name: "Unknown".to_string(),
            systems: Vec::new(),
            jump_gates: Vec::new(),
            stations: Vec::new(),
            celestial_bodies: Vec::new(),
            buildable_ship_types: Vec::new(),
        }
    }
}

/// Typed ship input built by the external adapter. Defaults remain exposed
/// so adapters and tests share the same fallback values without parsing maps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShipInput {
    pub is_player: bool,
    pub ship_type_name: String,
    pub max_shield: f64,
    pub max_armor: f64,
    pub max_hull: f64,
    pub current_shield: Option<f64>,
    pub current_armor: Option<f64>,
    pub current_hull: Option<f64>,
    pub cap_max: f64,
    pub cap_recharge_per_tick: f64,
}

pub fn default_max_shield() -> f64 {
    200.0
}

pub fn default_max_armor() -> f64 {
    150.0
}

pub fn default_max_hull() -> f64 {
    150.0
}

pub fn default_cap_max() -> f64 {
    500.0
}

pub fn default_cap_recharge() -> f64 {
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

#[derive(Debug, Clone, PartialEq)]
pub struct ShipState {
    pub ship_type_name: String,
    pub is_player: bool,
    pub cap_current: f64,
    pub cap_max: f64,
    pub cap_recharge_per_tick: f64,
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

/// What `destroy_ship` reports back. The only transition whose caller needs
/// more than one bit: the client frees the scene node (`destroyed`) and then
/// shows a duel result that depends on *whose* ship it was
/// (`destroyed_player` -> defeat, `destroyed_opponent` -> victory).
///
/// The sibling transitions (`register_ship`, `remove_ship`, `apply_hp_event`)
/// used to return structs of the same shape, but every field beyond the first
/// went unread by both the Godot client and the tests -- they reported values
/// that already drove internal state as locals. Reporting outward is not the
/// same as computing internally, and only the latter was load-bearing, so
/// those structs are gone and their methods return the one bit that was
/// actually consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DestructionOutcome {
    pub destroyed: bool,
    pub destroyed_player: bool,
    pub destroyed_opponent: bool,
}

/// One typed state transition accepted by [`WorldSessionState`].
///
/// The wire/Godot adapter converts external schemas into these plain values;
/// the state module remains independent from `dawn-wire` and Godot.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldSessionUpdate {
    InitialState {
        navigation: NavigationInput,
        ships: Vec<ShipRegistration>,
        connection_ship_id: i64,
    },
    ShipEntered {
        ship: ShipRegistration,
        connection_ship_id: i64,
    },
    ShipSpawned {
        ship_id: i64,
        connection_ship_id: i64,
    },
    ShipLeft {
        ship_id: i64,
        clear_lock: bool,
    },
    ShipDestroyed {
        ship_id: i64,
    },
    HealthChanged {
        ship_id: i64,
        shield: f64,
        armor: f64,
        hull: f64,
    },
    TargetLocked {
        locker_id: i64,
        target_id: i64,
    },
    LockLost {
        locker_id: i64,
        target_id: i64,
    },
    Docked {
        ship_id: i64,
        station_id: i64,
        station_name: String,
        tick: i64,
    },
    Undocked {
        ship_id: i64,
        tick: i64,
    },
    SystemChanged {
        ship_id: i64,
        to_system: i64,
    },
    Tick {
        tick: i64,
    },
    PlayerLoadout {
        active_ship_id: Option<i64>,
        docked_station_id: Option<i64>,
        docked_station_name: Option<String>,
        tick: i64,
    },
    /// A typed event whose only client-state effect is event accounting.
    ObservedEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShipRegistration {
    pub ship_id: i64,
    pub ship: ShipInput,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorldSessionEffect {
    None,
    InitialState {
        player_ship_id: i64,
    },
    ShipRegistered {
        registered: bool,
        became_player: bool,
    },
    ShipRemoved {
        removed: bool,
    },
    ShipDestroyed(DestructionOutcome),
    LockChanged {
        changed: bool,
    },
    DockState {
        accepted: bool,
    },
    SystemChanged {
        name: Option<String>,
    },
    TickAdvanced {
        ticks_elapsed: i64,
    },
    PlayerLoadout {
        active_changed: bool,
        dock_changed: bool,
    },
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
    pub fn apply_update(
        &mut self,
        update: WorldSessionUpdate,
        loadout: Option<&mut PlayerLoadoutMsg>,
    ) -> WorldSessionEffect {
        let counts_as_event = !matches!(
            update,
            WorldSessionUpdate::InitialState { .. } | WorldSessionUpdate::PlayerLoadout { .. }
        );
        if counts_as_event {
            self.increment_event_count();
        }

        match update {
            WorldSessionUpdate::InitialState {
                navigation,
                ships,
                connection_ship_id,
            } => {
                self.reset();
                self.ingest_navigation(navigation);
                for registration in ships {
                    self.register_ship(registration.ship_id, registration.ship, connection_ship_id);
                }
                WorldSessionEffect::InitialState {
                    player_ship_id: self.player_ship_id,
                }
            }
            WorldSessionUpdate::ShipEntered {
                ship,
                connection_ship_id,
            } => {
                let registered = !self.has_ship(ship.ship_id);
                let became_player = self.register_ship(ship.ship_id, ship.ship, connection_ship_id);
                WorldSessionEffect::ShipRegistered {
                    registered,
                    became_player,
                }
            }
            WorldSessionUpdate::ShipSpawned {
                ship_id,
                connection_ship_id,
            } => {
                let registered = !self.has_ship(ship_id);
                let became_player = self.register_ship(
                    ship_id,
                    ShipInput {
                        max_shield: default_max_shield(),
                        max_armor: default_max_armor(),
                        max_hull: default_max_hull(),
                        cap_max: default_cap_max(),
                        cap_recharge_per_tick: default_cap_recharge(),
                        ..ShipInput::default()
                    },
                    connection_ship_id,
                );
                WorldSessionEffect::ShipRegistered {
                    registered,
                    became_player,
                }
            }
            WorldSessionUpdate::ShipLeft {
                ship_id,
                clear_lock,
            } => WorldSessionEffect::ShipRemoved {
                removed: self.remove_ship(ship_id, clear_lock),
            },
            WorldSessionUpdate::ShipDestroyed { ship_id } => {
                WorldSessionEffect::ShipDestroyed(self.destroy_ship(ship_id))
            }
            WorldSessionUpdate::HealthChanged {
                ship_id,
                shield,
                armor,
                hull,
            } => {
                self.apply_hp_event(ship_id, shield, armor, hull);
                WorldSessionEffect::None
            }
            WorldSessionUpdate::TargetLocked {
                locker_id,
                target_id,
            } => WorldSessionEffect::LockChanged {
                changed: self.apply_target_locked(locker_id, target_id),
            },
            WorldSessionUpdate::LockLost {
                locker_id,
                target_id,
            } => WorldSessionEffect::LockChanged {
                changed: self.apply_lock_lost(locker_id, target_id),
            },
            WorldSessionUpdate::Docked {
                ship_id,
                station_id,
                station_name,
                tick,
            } => WorldSessionEffect::DockState {
                accepted: self.apply_dock_event(ship_id, station_id, station_name, tick),
            },
            WorldSessionUpdate::Undocked { ship_id, tick } => WorldSessionEffect::DockState {
                accepted: self.apply_undock_event(ship_id, tick),
            },
            WorldSessionUpdate::SystemChanged { ship_id, to_system } => {
                WorldSessionEffect::SystemChanged {
                    name: self.system_changed(ship_id, to_system),
                }
            }
            WorldSessionUpdate::Tick { tick } => WorldSessionEffect::TickAdvanced {
                ticks_elapsed: self.advance_tick_from_event(tick, loadout),
            },
            WorldSessionUpdate::PlayerLoadout {
                active_ship_id,
                docked_station_id,
                docked_station_name,
                tick,
            } => {
                let requested_ship_id = active_ship_id.unwrap_or(-1);
                let active_changed = requested_ship_id != self.player_ship_id
                    && (requested_ship_id < 0 || self.has_ship(requested_ship_id));
                if active_changed {
                    self.set_player_ship_id(requested_ship_id);
                }
                let station_id = docked_station_id.unwrap_or(-1);
                let dock_changed = self.apply_dock_fitting(
                    station_id,
                    docked_station_name.unwrap_or_default(),
                    tick,
                );
                WorldSessionEffect::PlayerLoadout {
                    active_changed,
                    dock_changed,
                }
            }
            WorldSessionUpdate::ObservedEvent => WorldSessionEffect::None,
        }
    }

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

    pub fn station_name(&self, station_id: i64) -> String {
        self.stations
            .iter()
            .find(|station| station.station_id == station_id)
            .map(|station| station.name.clone())
            .unwrap_or_else(|| format!("Station #{station_id}"))
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
        let Some(ship) = self.ships.get(&player_ship_id).cloned() else {
            return;
        };
        let Some(health) = self.ship_hp.get(&player_ship_id).copied() else {
            return;
        };
        self.player_ship_type_name = ship.ship_type_name;
        self.player_health = health;
        self.cap_current = ship.cap_current;
        self.cap_max = ship.cap_max;
        self.cap_recharge = ship.cap_recharge_per_tick;
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
    ) -> bool {
        if self.ships.contains_key(&ship_id) {
            return false;
        }

        let health = health_from_ship_input(&input);
        self.ship_hp.insert(ship_id, health);
        self.ships.insert(
            ship_id,
            ShipState {
                ship_type_name: input.ship_type_name.clone(),
                is_player: input.is_player,
                cap_current: input.cap_max,
                cap_max: input.cap_max,
                cap_recharge_per_tick: input.cap_recharge_per_tick,
            },
        );

        if ship_id == connection_ship_id && self.player_ship_id < 0 {
            self.set_player_ship_id(ship_id);
            return true;
        }
        if input.is_player && !self.opponent_ship_ids.contains(&ship_id) {
            self.opponent_ship_ids.push(ship_id);
        }
        false
    }

    /// Drops `ship_id` from the session. Returns whether it was there to drop
    /// -- the caller uses that to decide whether to free the scene node.
    pub fn remove_ship(&mut self, ship_id: i64, clear_lock: bool) -> bool {
        if self.ships.remove(&ship_id).is_none() {
            return false;
        }
        self.ship_hp.remove(&ship_id);
        if ship_id == self.player_ship_id {
            self.player_ship_id = -1;
        }
        remove_id(&mut self.opponent_ship_ids, ship_id);
        if clear_lock && ship_id == self.player_lock_target {
            self.player_lock_target = -1;
        }
        true
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

    /// Records a DamageTaken/RepairApplied update. Returns nothing: the
    /// caller passed `ship_id` in and already has it for its own visual
    /// feedback, and neither of the other two values the old
    /// `HealthEventOutcome` reported (`changed_player`, `has_ship`) had a
    /// reader on either side of the Godot boundary.
    pub fn apply_hp_event(&mut self, ship_id: i64, shield: f64, armor: f64, hull: f64) {
        if !self.ships.contains_key(&ship_id) {
            return;
        }
        let health = self.ship_hp.entry(ship_id).or_default();
        health.shield = shield;
        health.armor = armor;
        health.hull = hull;
        if ship_id == self.player_ship_id {
            self.player_health.shield = shield;
            self.player_health.armor = armor;
            self.player_health.hull = hull;
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
        let cap_current = match loadout {
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
        self.cap_current = cap_current;
        if let Some(ship) = self.ships.get_mut(&self.player_ship_id) {
            ship.cap_current = cap_current;
        }
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

        assert!(result, "the connection's own ship becomes the player ship");
        assert_eq!(state.player_ship_id(), 11);
        assert_eq!(state.player_ship_type_name(), "Magpie");
        assert_eq!(state.player_health().shield, 80.0);
        assert_eq!(state.cap_current(), 55.0);
    }

    #[test]
    fn switching_to_a_registered_ship_refreshes_player_projection() {
        let mut state = WorldSessionState::default();
        state.register_ship(11, ship(true), 11);

        let mut second = ship(false);
        second.ship_type_name = "Venture".to_string();
        second.max_shield = 250.0;
        second.max_armor = 180.0;
        second.max_hull = 120.0;
        second.current_shield = Some(210.0);
        second.current_armor = Some(160.0);
        second.current_hull = Some(110.0);
        second.cap_max = 80.0;
        second.cap_recharge_per_tick = 4.0;
        state.register_ship(22, second, 11);

        state.cap_current = 17.0;
        state.ships.get_mut(&11).unwrap().cap_current = 17.0;
        state.set_player_ship_id(22);

        assert_eq!(state.player_ship_type_name(), "Venture");
        assert_eq!(state.player_health().shield, 210.0);
        assert_eq!(state.player_health().max_shield, 250.0);
        assert_eq!(state.cap_current(), 80.0);
        assert_eq!(state.cap_max(), 80.0);
        assert_eq!(state.cap_recharge(), 4.0);

        state.cap_current = 31.0;
        state.ships.get_mut(&22).unwrap().cap_current = 31.0;
        state.set_player_ship_id(11);

        assert_eq!(state.player_ship_type_name(), "Magpie");
        assert_eq!(state.player_health().shield, 80.0);
        assert_eq!(state.cap_current(), 17.0);
        assert_eq!(state.cap_max(), 55.0);
        assert_eq!(state.cap_recharge(), 3.0);
    }

    #[test]
    fn removing_an_aoi_ship_can_preserve_the_lock_target() {
        let mut state = WorldSessionState::default();
        state.register_ship(11, ship(true), 99);
        state.player_ship_id = 99;
        state.register_ship(42, ship(false), 99);
        state.player_lock_target = 42;

        let result = state.remove_ship(42, false);

        assert!(result);
        assert!(!state.has_ship(42));
        assert_eq!(state.player_lock_target(), 42);
    }

    #[test]
    fn stale_dock_context_does_not_overwrite_a_newer_one() {
        let mut state = WorldSessionState {
            player_ship_id: 7,
            ..WorldSessionState::default()
        };
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

    #[test]
    fn typed_initial_state_is_the_authoritative_ingestion_path() {
        let mut state = WorldSessionState::default();
        let effect = state.apply_update(
            WorldSessionUpdate::InitialState {
                navigation: NavigationInput {
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
                },
                ships: vec![ShipRegistration {
                    ship_id: 11,
                    ship: ship(true),
                }],
                connection_ship_id: 11,
            },
            None,
        );

        assert_eq!(
            effect,
            WorldSessionEffect::InitialState { player_ship_id: 11 }
        );
        assert_eq!(state.current_system_name(), "Alpha");
        assert_eq!(state.gates()[0].position[0], 149_597_870_710.0);
        assert_eq!(state.player_ship_id(), 11);
    }

    #[test]
    fn typed_event_updates_cover_lifecycle_health_lock_and_docking() {
        let mut state = WorldSessionState::default();
        state.apply_update(
            WorldSessionUpdate::ShipEntered {
                ship: ShipRegistration {
                    ship_id: 11,
                    ship: ship(true),
                },
                connection_ship_id: 11,
            },
            None,
        );
        state.apply_update(
            WorldSessionUpdate::HealthChanged {
                ship_id: 11,
                shield: 10.0,
                armor: 20.0,
                hull: 30.0,
            },
            None,
        );
        assert_eq!(state.player_health().hull, 30.0);
        assert!(matches!(
            state.apply_update(
                WorldSessionUpdate::TargetLocked {
                    locker_id: 11,
                    target_id: 42,
                },
                None,
            ),
            WorldSessionEffect::LockChanged { changed: true }
        ));
        assert_eq!(state.player_lock_target(), 42);
        assert!(matches!(
            state.apply_update(
                WorldSessionUpdate::Docked {
                    ship_id: 11,
                    station_id: 3,
                    station_name: "Forge".to_string(),
                    tick: 8,
                },
                None,
            ),
            WorldSessionEffect::DockState { accepted: true }
        ));
        assert!(state.is_docked());
        assert!(matches!(
            state.apply_update(
                WorldSessionUpdate::ShipLeft {
                    ship_id: 11,
                    clear_lock: true,
                },
                None,
            ),
            WorldSessionEffect::ShipRemoved { removed: true }
        ));
        assert!(!state.has_ship(11));
        assert_eq!(state.event_count(), 5);
    }

    #[test]
    fn typed_player_loadout_rejects_unknown_active_ship_and_stale_dock_state() {
        let mut state = WorldSessionState::default();
        state.apply_update(
            WorldSessionUpdate::ShipEntered {
                ship: ShipRegistration {
                    ship_id: 11,
                    ship: ship(true),
                },
                connection_ship_id: 11,
            },
            None,
        );
        state.apply_update(
            WorldSessionUpdate::Docked {
                ship_id: 11,
                station_id: 3,
                station_name: "New".to_string(),
                tick: 20,
            },
            None,
        );

        let effect = state.apply_update(
            WorldSessionUpdate::PlayerLoadout {
                active_ship_id: Some(99),
                docked_station_id: Some(2),
                docked_station_name: Some("Stale".to_string()),
                tick: 19,
            },
            None,
        );
        assert_eq!(
            effect,
            WorldSessionEffect::PlayerLoadout {
                active_changed: false,
                dock_changed: false,
            }
        );
        assert_eq!(state.player_ship_id(), 11);
        assert_eq!(state.docked_station_id(), 3);
    }

    #[test]
    fn typed_health_update_ignores_unknown_ship() {
        let mut state = WorldSessionState::default();

        state.apply_update(
            WorldSessionUpdate::HealthChanged {
                ship_id: 99,
                shield: 1.0,
                armor: 2.0,
                hull: 3.0,
            },
            None,
        );

        assert!(!state.ship_hp().contains_key(&99));
    }
}
