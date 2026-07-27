//! Load game data from TOML files at server startup.
//!
//! Each submodule handles one data category. Falls back to built-in defaults
//! when the file is absent or fails to parse.

mod modules;
mod ship_types;
pub use modules::load_modules;
pub use ship_types::load_ship_types;
