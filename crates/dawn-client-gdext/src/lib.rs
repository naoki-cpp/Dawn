//! GDExtension binding exposing `dawn-client-core` to the Godot client
//! (ADR-0040). This crate is a thin adapter between Godot's Variant/GString/
//! Dictionary types and `dawn-client-core`'s plain Rust types -- it holds no
//! domain logic of its own.

use godot::prelude::*;

mod client_command_gd;
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
pub use item_row_gd::ItemRow;
pub use loadout_gd::PlayerLoadout;
pub use module_row_gd::ModuleRow;
pub use owned_ship_row_gd::OwnedShipRow;
pub use server_message_gd::ServerMessageDecoder;
pub use ship_motion_gd::ShipMotion;
pub use world_session_gd::WorldSession;
pub use world_space_gd::WorldSpace;

struct DawnClientGdext;

#[gdextension]
unsafe impl ExtensionLibrary for DawnClientGdext {}
