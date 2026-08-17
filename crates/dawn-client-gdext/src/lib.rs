//! GDExtension binding exposing `dawn-client-core` to the Godot client
//! (ADR-0040). This crate converts `dawn-protocol` messages into typed client
//! facts and Godot presentation records. Every inbound state mutation is
//! applied before the single presentation dispatch seam invokes GDScript.

use godot::prelude::*;

mod client_action_gd;
mod client_command_gd;
mod client_rules_gd;
mod item_identity_gd;
mod item_row_gd;
mod loadout_gd;
mod module_activation_intent_gd;
mod module_row_gd;
mod owned_ship_row_gd;
mod presentation_gd;
mod server_message_gd;
mod server_message_validation;
mod session_record_gd;
mod ship_motion_gd;
mod world_session_gd;
mod world_space_gd;

#[cfg(test)]
mod client_outcome {
    pub(crate) use crate::server_message_validation::validate_player_loadout_godot_ranges;
}

pub use client_action_gd::{ClientAction, ClientInteraction};
pub use client_command_gd::{ClientCommand, ClientCommandResult};
pub use client_rules_gd::ClientRules;
pub use item_identity_gd::ItemIdentity;
pub use item_row_gd::ItemRow;
pub use loadout_gd::PlayerLoadout;
pub use module_activation_intent_gd::ModuleActivationIntent;
pub use module_row_gd::ModuleRow;
pub use owned_ship_row_gd::OwnedShipRow;
pub use presentation_gd::{
    InitialStatePresentation, MarketOrder, MarketSnapshot, MotionCorrectionPresentation,
    ShipPresentation,
};
pub use server_message_gd::{ServerMessageDecoder, ServerMessageOutcome};
pub use ship_motion_gd::ShipMotion;
pub use world_session_gd::WorldSession;
pub use world_space_gd::WorldSpace;

struct DawnClientGdext;

#[gdextension]
unsafe impl ExtensionLibrary for DawnClientGdext {}
