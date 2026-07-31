//! Authoritative module and ship-type game-data catalog.
//!
//! Production balance lives in `data/modules.toml` and `data/ship_types.toml`.
//! Every server runtime reaches this strict catalog implementation; there is no
//! built-in balance fallback.

mod module_file;
mod ship_type_file;
mod validation;

use crate::node::SimulationNode;
use dawn_core::fitting::ModuleDefinition;
use dawn_core::ship_type::ShipTypeDefinition;
use dawn_event_store::store::EventStore;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) use module_file::load_modules_file;
pub(crate) use ship_type_file::load_ship_types_file;
use validation::validate_required_ids;

pub const PRODUCTION_MODULES_PATH: &str = "data/modules.toml";
pub const PRODUCTION_SHIP_TYPES_PATH: &str = "data/ship_types.toml";

static RUNTIME_CATALOG: OnceLock<Result<GameDataCatalog, CatalogError>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct GameDataCatalog {
    modules: Vec<ModuleDefinition>,
    ship_types: Vec<ShipTypeDefinition>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("failed to read required {category} game data from '{path}': {source}")]
    Read {
        category: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse required {category} game data from '{path}': {source}")]
    Parse {
        category: &'static str,
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("invalid {category} game data in '{path}': {message}")]
    Validation {
        category: &'static str,
        path: PathBuf,
        message: String,
    },
}

impl GameDataCatalog {
    /// Load required production data from the process working directory.
    pub fn load_production() -> Result<Self, CatalogError> {
        Self::load_from_paths(PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH)
    }

    pub fn load_from_paths(
        modules_path: impl AsRef<Path>,
        ship_types_path: impl AsRef<Path>,
    ) -> Result<Self, CatalogError> {
        let modules_path = modules_path.as_ref();
        let ship_types_path = ship_types_path.as_ref();
        let modules = load_modules_file(modules_path)?;
        let ship_types = load_ship_types_file(ship_types_path)?;
        validate_required_ids(&modules, &ship_types, modules_path, ship_types_path)?;
        Ok(Self {
            modules,
            ship_types,
        })
    }

    pub fn modules(&self) -> &[ModuleDefinition] {
        &self.modules
    }

    pub fn ship_types(&self) -> &[ShipTypeDefinition] {
        &self.ship_types
    }

    pub fn into_modules(self) -> Vec<ModuleDefinition> {
        self.modules
    }

    pub fn into_ship_types(self) -> Vec<ShipTypeDefinition> {
        self.ship_types
    }

    pub fn register_into<S: EventStore>(&self, node: &mut SimulationNode<S>) {
        for definition in &self.modules {
            node.register_module(definition.clone());
        }
        for definition in &self.ship_types {
            node.register_ship_type(definition.clone());
        }
    }

    /// Load the runtime-relative catalog, resolving to the source checkout only
    /// when both packaged files are absent and the checkout is actually present.
    /// This keeps deployed startup tied to its `data/` directory while allowing
    /// tests that temporarily change their working directory to reuse repository data.
    pub fn load_runtime() -> Result<Self, CatalogError> {
        let runtime_modules = Path::new(PRODUCTION_MODULES_PATH);
        let runtime_ship_types = Path::new(PRODUCTION_SHIP_TYPES_PATH);

        if !runtime_modules.exists() && !runtime_ship_types.exists() {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let source_modules = root.join(PRODUCTION_MODULES_PATH);
            let source_ship_types = root.join(PRODUCTION_SHIP_TYPES_PATH);
            if source_modules.exists() && source_ship_types.exists() {
                return Self::load_from_paths(source_modules, source_ship_types);
            }
        }

        Self::load_from_paths(runtime_modules, runtime_ship_types)
    }
}

/// Return the process-wide runtime catalog, loading and validating both TOML files once.
///
/// Every production accessor shares this exact catalog instance, so module and ship-type
/// definitions cannot come from different reads or different fallback rules.
pub fn runtime_catalog() -> Result<&'static GameDataCatalog, &'static CatalogError> {
    RUNTIME_CATALOG
        .get_or_init(GameDataCatalog::load_runtime)
        .as_ref()
}

#[cfg(test)]
impl GameDataCatalog {
    pub(crate) fn load_repository_data() -> Result<Self, CatalogError> {
        Self::load_runtime()
    }
}

pub(super) fn read_required(category: &'static str, path: &Path) -> Result<String, CatalogError> {
    std::fs::read_to_string(path).map_err(|source| CatalogError::Read {
        category,
        path: path.to_path_buf(),
        source,
    })
}

pub(super) fn parse_required<T: DeserializeOwned>(
    category: &'static str,
    path: &Path,
    source: &str,
) -> Result<T, CatalogError> {
    toml::from_str(source).map_err(|source| CatalogError::Parse {
        category,
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

#[cfg(test)]
mod tests;
