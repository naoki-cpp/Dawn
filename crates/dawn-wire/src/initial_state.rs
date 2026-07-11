//! `InitialState` wire schema (ADR-0042 stage 2b).
//!
//! Sent once per connection (fresh spawn, resume, or jump handoff) to give the
//! client the navigation map plus every ship it can currently see. Absolute
//! positions here are f64 (ADR-0029) -- [`PosJson`](crate::PosJson) is f32 and
//! is used only for client command targets, so this module has its own
//! `AbsPosJson`.

use dawn_core::CelestialBodyKind;
use serde::{Deserialize, Serialize};

/// Absolute (Sector-frame, f64) position (ADR-0029). Distinct from
/// [`crate::PosJson`] (f32), which carries client-authored command targets
/// rather than server-authoritative absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AbsPosJson {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Per-ship state: position, stats, hull, ownership. Shared by `InitialState`
/// and `AoiEnter` (ADR-0019).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipStateJson {
    pub ship_id: u64,
    pub ship_type_name: String,
    pub position: AbsPosJson,
    pub max_shield: f32,
    pub max_armor: f32,
    pub max_hull: f32,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub cap_max: f32,
    pub cap_recharge_per_tick: f32,
    pub is_player: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CelestialBodyJson {
    pub id: u32,
    pub kind: CelestialBodyKind,
    pub name: String,
    pub position: AbsPosJson,
    pub radius: f32,
    pub spectral_type: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemJson {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpGateJson {
    pub gate_id: u32,
    pub position: AbsPosJson,
    pub activation_radius: f32,
    pub to_system_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationJson {
    pub station_id: u32,
    pub name: String,
    pub position: AbsPosJson,
    pub docking_radius: f32,
}

/// Buildable Packaged Ship catalog entry (ADR-0034 9B). Static registry data,
/// sent once alongside the rest of `InitialState` rather than as its own
/// message type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildableShipTypeJson {
    pub ship_type_id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialStateJson {
    pub ships: Vec<ShipStateJson>,
    pub system_name: String,
    pub systems: Vec<SystemJson>,
    pub jump_gates: Vec<JumpGateJson>,
    pub stations: Vec<StationJson>,
    pub celestial_bodies: Vec<CelestialBodyJson>,
    pub buildable_ship_types: Vec<BuildableShipTypeJson>,
}
