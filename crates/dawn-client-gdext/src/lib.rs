//! GDExtension binding exposing `dawn-client-core` to the Godot client
//! (ADR-0040). This crate adapts Godot's Variant/GString/Dictionary types and
//! projects `dawn-wire` messages into typed client outcomes; domain state and
//! rules remain in the Godot-independent client/core crates. Raw frame bytes
//! are retained only where an existing typed decoder consumes them directly.

use godot::prelude::*;

mod client_command_gd;
mod client_outcome;
mod client_rules_gd;
mod item_identity_gd;
mod item_row_gd;
mod json_variant;
mod loadout_gd;
mod module_row_gd;
mod navigation_gd;
mod owned_ship_row_gd;
mod server_message_gd;
mod session_record_gd;
mod ship_gd;
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
pub use server_message_gd::{ServerEventOutcome, ServerMessageDecoder, ServerMessageOutcome};
pub use ship_motion_gd::ShipMotion;
pub use world_session_gd::WorldSession;
pub use world_space_gd::WorldSpace;

struct DawnClientGdext;

#[gdextension]
unsafe impl ExtensionLibrary for DawnClientGdext {}
