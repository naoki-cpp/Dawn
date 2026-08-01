//! GDExtension binding exposing `dawn-client-core` to the Godot client
//! (ADR-0040). This crate converts `dawn-wire` messages into typed client
//! updates and Godot presentation records. Every inbound state mutation is
//! applied to `WorldSessionState` before presentation callbacks run; domain
//! state and rules remain in the Godot-independent client/core crates. Raw
//! frame bytes are retained only where an existing typed decoder consumes
//! them directly.

use godot::prelude::*;

mod client_command_gd;
mod client_outcome;
mod client_rules_gd;
mod item_identity_gd;
mod item_row_gd;
mod json_variant;
mod loadout_gd;
mod module_row_gd;
mod owned_ship_row_gd;
mod presentation_gd;
mod server_message_gd;
mod session_record_gd;
mod ship_motion_gd;
mod world_session_gd;
mod world_space_gd;

pub use client_command_gd::{ClientCommand, ClientMessageDecoder};
pub use client_rules_gd::ClientRules;
pub use item_identity_gd::ItemIdentity;
pub use item_row_gd::ItemRow;
pub use loadout_gd::PlayerLoadout;
pub use module_row_gd::ModuleRow;
pub use owned_ship_row_gd::OwnedShipRow;
pub use presentation_gd::{
    InitialStatePresentation, MarketOrder, MarketSnapshot, MotionCorrectionPresentation,
    ShipPresentation,
};
pub use server_message_gd::{ServerEventOutcome, ServerMessageDecoder, ServerMessageOutcome};
pub use ship_motion_gd::ShipMotion;
pub use world_session_gd::WorldSession;
pub use world_space_gd::WorldSpace;

struct DawnClientGdext;

#[gdextension]
unsafe impl ExtensionLibrary for DawnClientGdext {}
