//! Godot-independent client-side domain model for Dawn (ADR-0039).
//!
//! This crate owns the client's richer concept behind the server's
//! `PlayerLoadout` wire message: fitted modules, ship/station inventory,
//! dock context, and capacitor-cycle runtime simulation. It depends only on
//! `dawn-core` so its types can be shared with the server's command/event
//! types without a JSON round-trip re-deriving the same shape twice.
//!
//! Nothing here talks to Godot, a network socket, or the filesystem --
//! callers (a future GDExtension binding, or this crate's own tests) own
//! those concerns and hand this crate plain values.
//!
//! # Example
//!
//! ```
//! use dawn_client_core::PlayerLoadoutMsg;
//!
//! let json = r#"{
//!     "tick": 42,
//!     "modules": [],
//!     "inventory": [],
//!     "station_inventory": [],
//!     "docked_station_id": null,
//!     "docked_station_name": null,
//!     "slot_capacity": {"High": 4, "Mid": 4, "Low": 4, "Rig": 3},
//!     "active_ship_id": 7,
//!     "owned_ships": []
//! }"#;
//!
//! let loadout: PlayerLoadoutMsg = serde_json::from_str(json).unwrap();
//! assert!(!loadout.is_docked());
//! assert_eq!(loadout.active_ship_id, Some(7));
//! ```

mod item_row;
mod loadout;
mod module_row;
mod motion;
mod world_session;
mod world_space;

pub use item_row::{ItemRow, ItemType};
pub use loadout::{
    simulate_modules_capacitor_ticks, ActivationIntent, OwnedShipRow, PlayerLoadoutMsg,
    SlotCapacity,
};
pub use module_row::{ModuleKind, ModuleRow, StatDelta};
pub use motion::{MotionInput, MotionPredictor, MotionProfile, MotionState};
pub use world_session::{
    BuildableShipTypeInput, BuildableShipTypeRecord, CelestialBodyInput, CelestialBodyRecord,
    DestructionOutcome, GateInput, GateRecord, HealthEventInput, HealthEventOutcome, HealthState,
    NavigationInput, PositionInput, RegistrationOutcome, RemovalOutcome, ShipInput, ShipState,
    StationInput, StationRecord, SystemNameInput, WorldSessionState,
};
pub use world_space::{WorldSpace, REBASE_THRESHOLD, WORLD_SCALE};
