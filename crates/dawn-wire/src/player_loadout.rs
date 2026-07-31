//! `PlayerLoadout` wire schema (ADR-0042 stage 2a).
//!
//! The server sends this after Welcome/InitialState and again after every
//! Fit/Unfit (ADR-0032). Building block types (`ModuleRowWire`/`ItemRowWire`/
//! `SlotCapacityWire`/`OwnedShipRowWire`) mirror exactly what
//! `dawn-sector`'s `player_loadout_projection.rs` used to hand-assemble via
//! `serde_json::json!`, and what `dawn-client-core`'s `PlayerLoadoutMsg`/
//! `ModuleRow`/`ItemRow`/`SlotCapacity`/`OwnedShipRow` already expect to
//! parse (that crate keeps its own decoupled, forward-compatible mirror --
//! e.g. a `Unknown` fallback for `ModuleKind` -- so this crate's types stay
//! the exact current schema without needing that tolerance).

use crate::ItemWire;
use dawn_core::{ModuleKind, StatDelta};
use serde::{Deserialize, Serialize};

/// One row of `PlayerLoadout`'s `modules` array (a fitted module slot).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleRowWire {
    pub slot: String,
    pub index: u32,
    pub module_id: u32,
    pub name: String,
    pub kind: ModuleKind,
    pub is_active: bool,
    pub is_active_module: bool,
    pub cap_cost_per_cycle: f32,
    pub cycle_time_ticks: u64,
    pub stat_delta: StatDelta,
}

/// One row of `PlayerLoadout`'s `inventory`/`station_inventory` arrays.
/// `item_id` preserves the Item variant and carries only its owned ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRowWire {
    pub item_id: ItemWire,
    pub name: String,
    pub kind: String,
    pub slot: String,
    pub count: u64,
}

/// Per-slot-kind fitted module capacity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlotCapacityWire {
    pub high: u8,
    pub mid: u8,
    pub low: u8,
    pub rig: u8,
}

/// One row of `PlayerLoadout`'s `owned_ships` array (ADR-0037's full
/// owned-ship roster, active or not).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedShipRowWire {
    pub ship_id: u64,
    pub ship_type_id: Option<u32>,
    pub ship_type_name: Option<String>,
    pub docked_station_id: Option<u32>,
    pub is_active: bool,
}

/// The full `PlayerLoadout` wire message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerLoadoutWire {
    pub tick: u64,
    pub modules: Vec<ModuleRowWire>,
    pub inventory: Vec<ItemRowWire>,
    pub station_inventory: Vec<ItemRowWire>,
    pub docked_station_id: Option<u32>,
    pub docked_station_name: Option<String>,
    pub slot_capacity: SlotCapacityWire,
    pub active_ship_id: Option<u64>,
    pub owned_ships: Vec<OwnedShipRowWire>,
}
