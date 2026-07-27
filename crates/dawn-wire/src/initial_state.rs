//! `InitialState` wire schema (ADR-0042 stage 2b).
//!
//! Sent once per connection (fresh spawn, resume, or jump handoff) to give the
//! client the navigation map plus every ship it can currently see. Absolute
//! positions here are f64 (ADR-0029) -- [`PosWire`](crate::PosWire) is also f64
//! is used only for client command targets, so this module has its own
//! `AbsPosWire`.

use dawn_core::{AbsolutePosition, CelestialBodyKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::VelWire;

/// Absolute (Sector-frame, f64) position (ADR-0029). Distinct from
/// [`crate::PosWire`] (f64), which carries client-authored command targets
/// rather than server-authoritative absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AbsPosWire {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl From<AbsolutePosition> for AbsPosWire {
    fn from(position: AbsolutePosition) -> Self {
        Self {
            x: position[0],
            y: position[1],
            z: position[2],
        }
    }
}

/// Per-ship state: position, stats, hull, ownership. Shared by `InitialState`
/// and `AoiEnter` (ADR-0019).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipStateWire {
    pub ship_id: u64,
    pub ship_type_name: String,
    pub position: AbsPosWire,
    pub velocity: VelWire,
    pub max_speed: f64,
    pub mass: f64,
    pub inertia_modifier: f64,
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
pub struct CelestialBodyWire {
    pub id: u32,
    pub kind: CelestialBodyKind,
    pub name: String,
    pub position: AbsPosWire,
    pub radius: f64,
    pub spectral_type: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemWire {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpGateWire {
    pub gate_id: u32,
    pub position: AbsPosWire,
    pub activation_radius: f64,
    pub to_system_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationWire {
    pub station_id: u32,
    pub name: String,
    pub position: AbsPosWire,
    pub docking_radius: f64,
}

/// Buildable Packaged Ship catalog entry (ADR-0034 9B). Static registry data,
/// sent once alongside the rest of `InitialState` rather than as its own
/// message type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildableShipTypeWire {
    pub ship_type_id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialStateWire {
    pub ships: Vec<ShipStateWire>,
    pub system_name: String,
    pub systems: Vec<SystemWire>,
    pub jump_gates: Vec<JumpGateWire>,
    pub stations: Vec<StationWire>,
    pub celestial_bodies: Vec<CelestialBodyWire>,
    pub buildable_ship_types: Vec<BuildableShipTypeWire>,
}
