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
//! use dawn_core::{NodeId, ShipId};
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
//! assert_eq!(loadout.active_ship_id, Some(ShipId::new(NodeId(0), 7)));
//! ```

mod client_action;
mod client_rules;
mod client_state;
mod hud;
mod item_row;
mod loadout;
mod module_row;
mod motion;
mod ship_motion;
mod station_inventory;
mod world_session;
mod world_space;

pub use client_action::{
    ClientAction, ClientActionContext, ClientInteraction, ClientKey, ClientLocalAction, Selection,
};
pub use client_rules::ClientRules;
pub use client_state::{ClientFact, ClientState, ClientStateError, ShipLeaveReason};
pub use dawn_core::{ModuleKind, StatDelta};
pub use hud::{
    HudChangeSet, HudFrame, HudReadModel, HudSceneFacts, HudShipStatusPanel, HudSnapshot,
    HudStatsPanel, HudStatusPanel, HudTargetPanel,
};
pub use item_row::ItemRow;
pub use loadout::{
    simulate_modules_capacitor_ticks, ActivationIntent, OwnedShipRow, PlayerLoadoutMsg,
    SlotCapacity,
};
pub use module_row::ModuleRow;
pub use motion::{MotionInput, MotionPredictor, MotionProfile, MotionState};
pub use ship_motion::{MotionCommand, MotionDispatch, MotionFrame, ShipMotion};
pub use station_inventory::{
    FittedModuleRow, StationInventoryAction, StationInventoryColumn, StationInventoryContext,
    StationInventoryInteraction, StationInventoryLocalAction, StationInventoryRow,
};
pub use world_session::{
    default_cap_max, default_cap_recharge, default_max_armor, default_max_hull, default_max_shield,
    BuildableShipTypeInput, BuildableShipTypeRecord, CelestialBodyInput, CelestialBodyRecord,
    DestructionOutcome, GateInput, GateRecord, HealthState, NavigationInput, PositionInput,
    ShipInput, ShipRegistration, ShipState, StationInput, StationRecord, SystemNameInput,
    WorldSessionEffect, WorldSessionState,
};
pub use world_space::{WorldSpace, REBASE_THRESHOLD, WORLD_SCALE};
