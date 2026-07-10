//! GDExtension binding exposing `dawn-client-core` to the Godot client
//! (ADR-0040). This crate is a thin adapter between Godot's Variant/GString/
//! Dictionary types and `dawn-client-core`'s plain Rust types -- it holds no
//! domain logic of its own.

use godot::prelude::*;

mod item_row_gd;
mod loadout_gd;
mod module_row_gd;

pub use item_row_gd::ItemRow;
pub use loadout_gd::PlayerLoadout;
pub use module_row_gd::ModuleRow;

struct DawnClientGdext;

#[gdextension]
unsafe impl ExtensionLibrary for DawnClientGdext {}
