//! Backward-compatible game-data loading entry points.
//!
//! Both server runtimes reach the same strict [`crate::game_data::GameDataCatalog`]
//! implementation through these functions. The fallback arguments remain only
//! for source compatibility and are deliberately ignored.

use crate::game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH};
use dawn_core::fitting::ModuleDefinition;
use dawn_core::ship_type::ShipTypeDefinition;
use std::path::{Path, PathBuf};

/// Load the authoritative catalog and return its module definitions.
///
/// Missing, malformed, or invalid required data aborts startup instead of
/// selecting different built-in balance rules.
pub fn load_modules(path: &str, _fallback: Vec<ModuleDefinition>) -> Vec<ModuleDefinition> {
    GameDataCatalog::load_from_paths(
        resolve_catalog_path(path),
        resolve_catalog_path(PRODUCTION_SHIP_TYPES_PATH),
    )
    .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
    .into_modules()
}

/// Load the authoritative catalog and return its ship-type definitions.
///
/// Missing, malformed, or invalid required data aborts startup instead of
/// selecting different built-in balance rules.
pub fn load_ship_types(path: &str, _fallback: Vec<ShipTypeDefinition>) -> Vec<ShipTypeDefinition> {
    GameDataCatalog::load_from_paths(
        resolve_catalog_path(PRODUCTION_MODULES_PATH),
        resolve_catalog_path(path),
    )
    .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
    .into_ship_types()
}

fn resolve_catalog_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}
