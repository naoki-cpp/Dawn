//! Typed GDScript-facing views of the values `WorldSession` reports.
//!
//! These replace the untyped `Dictionary` returns the adapter used to build.
//! A `Dictionary` forces every caller to know key strings and re-assert types
//! at each read (`g.get("activation_radius", 0.0) as float`), which put the
//! record's shape in `main.gd`'s head instead of in one place; a typed class
//! puts it back here and lets GDScript dot-access it.
//!
//! Pure data carriers, so they live together rather than one per file --
//! unlike `ItemRow`/`ModuleRow`, which own parsing behaviour of their own.
//!
//! All of them are `init`-constructible with writable `#[var]` fields so
//! GDScript tests can build fixtures directly. That is deliberately not a
//! `from_dict` factory: a factory would re-introduce the key-string parsing
//! this module exists to remove, and would let a typo in a fixture silently
//! default instead of failing.

use dawn_client_core::{
    BuildableShipTypeRecord as CoreBuildableShipType, CelestialBodyRecord as CoreCelestialBody,
    DestructionOutcome as CoreDestructionOutcome, GateRecord as CoreGate, HealthState,
    StationRecord as CoreStation,
};
use godot::prelude::*;

/// A Jump Gate's navigation record (`InitialState`'s gate list).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct GateRecord {
    #[var]
    gate_id: i64,
    /// Canonical f64 server-space components. This remains a
    /// `PackedFloat64Array` until the final WorldSpace rendering conversion.
    #[var]
    position: PackedFloat64Array,
    #[var]
    activation_radius: f64,
    #[var]
    to_system_name: GString,
}

impl GateRecord {
    pub(crate) fn wrap(gate: &CoreGate) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            gate_id: gate.gate_id,
            position: PackedFloat64Array::from(gate.position),
            activation_radius: gate.activation_radius,
            to_system_name: (&gate.to_system_name).into(),
        })
    }
}

#[godot_api]
impl GateRecord {}

/// A Station's navigation record (`InitialState`'s station list).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct StationRecord {
    #[var]
    station_id: i64,
    #[var]
    name: GString,
    #[var]
    position: PackedFloat64Array,
    #[var]
    docking_radius: f64,
}

impl StationRecord {
    pub(crate) fn wrap(station: &CoreStation) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            station_id: station.station_id,
            name: (&station.name).into(),
            position: PackedFloat64Array::from(station.position),
            docking_radius: station.docking_radius,
        })
    }
}

#[godot_api]
impl StationRecord {}

/// A star or planet (ADR-0025).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct CelestialBodyRecord {
    #[var]
    body_id: i64,
    #[var]
    kind: GString,
    #[var]
    name: GString,
    #[var]
    position: PackedFloat64Array,
    #[var]
    radius: f64,
    #[var]
    spectral_type: f64,
}

impl CelestialBodyRecord {
    pub(crate) fn wrap(body: &CoreCelestialBody) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            body_id: body.body_id,
            kind: (&body.kind).into(),
            name: (&body.name).into(),
            position: PackedFloat64Array::from(body.position),
            radius: body.radius,
            spectral_type: body.spectral_type,
        })
    }
}

#[godot_api]
impl CelestialBodyRecord {}

/// A ship type the player may build at a Station (ADR-0034).
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct BuildableShipType {
    #[var]
    ship_type_id: i64,
    #[var]
    name: GString,
}

impl BuildableShipType {
    pub(crate) fn wrap(ship: &CoreBuildableShipType) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_type_id: ship.ship_type_id,
            name: (&ship.name).into(),
        })
    }
}

#[godot_api]
impl BuildableShipType {}

/// One ship's Shield/Armor/Hull layers with their maxima (ADR-0006).
///
/// `ship_id` is carried so a caller holding several of these can tell them
/// apart; `player_health()` returns the player's own with its ship id
/// already filled in.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ShipHealth {
    #[var]
    ship_id: i64,
    #[var]
    shield: f64,
    #[var]
    armor: f64,
    #[var]
    hull: f64,
    #[var]
    max_shield: f64,
    #[var]
    max_armor: f64,
    #[var]
    max_hull: f64,
}

impl ShipHealth {
    pub(crate) fn wrap(ship_id: i64, health: HealthState) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id,
            shield: health.shield,
            armor: health.armor,
            hull: health.hull,
            max_shield: health.max_shield,
            max_armor: health.max_armor,
            max_hull: health.max_hull,
        })
    }
}

#[godot_api]
impl ShipHealth {
    /// Field-value equality, mirroring `ModuleRow::equals` and for the same
    /// reason: `hud_surface.gd` diffs the target panel against last frame to
    /// skip repaints, and `WorldSession::ship_health` mints a fresh object
    /// every call, so Godot's default reference-identity `==` would report
    /// "changed" on every single frame.
    #[func]
    fn equals(&self, other: Gd<ShipHealth>) -> bool {
        let other = other.bind();
        self.ship_id == other.ship_id
            && self.shield == other.shield
            && self.armor == other.armor
            && self.hull == other.hull
            && self.max_shield == other.max_shield
            && self.max_armor == other.max_armor
            && self.max_hull == other.max_hull
    }
}

/// The player's capacitor pool (ADR-0011). Read as a unit by the HUD.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct CapacitorStatus {
    #[var]
    current: f64,
    #[var]
    max: f64,
    #[var]
    recharge: f64,
}

impl CapacitorStatus {
    pub(crate) fn wrap(current: f64, max: f64, recharge: f64) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            current,
            max,
            recharge,
        })
    }
}

#[godot_api]
impl CapacitorStatus {}

/// What `destroy_ship` reports: whether the ship was there to destroy, and
/// whose it was -- the HUD shows defeat for the player's own ship and
/// victory for a tracked opponent.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct DestructionOutcome {
    #[var]
    destroyed: bool,
    #[var]
    destroyed_player: bool,
    #[var]
    destroyed_opponent: bool,
}

impl DestructionOutcome {
    pub(crate) fn wrap(outcome: CoreDestructionOutcome) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            destroyed: outcome.destroyed,
            destroyed_player: outcome.destroyed_player,
            destroyed_opponent: outcome.destroyed_opponent,
        })
    }
}

#[godot_api]
impl DestructionOutcome {}
