//! Authoritative module and ship-type game-data catalog.
//!
//! Production balance lives in `data/modules.toml` and `data/ship_types.toml`.
//! A catalog is fully parsed and validated before it can be supplied to a
//! Sector. Definitions are immutable after construction, lookups are indexed,
//! and every observable iteration is ordered by the stable numeric ID.

mod module_file;
mod ship_type_file;
mod validation;

use dawn_core::fitting::{ModuleDefinition, ModuleId};
use dawn_core::ship_type::{ShipTypeDefinition, ShipTypeId};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;

pub(crate) use module_file::load_modules_file;
pub(crate) use ship_type_file::load_ship_types_file;
use validation::validate_required_ids;

pub const PRODUCTION_MODULES_PATH: &str = "data/modules.toml";
pub const PRODUCTION_SHIP_TYPES_PATH: &str = "data/ship_types.toml";

/// Complete, validated and immutable game definitions used by a Sector.
///
/// The ordered slices are the canonical observable views. The maps are built
/// once from the same definitions and provide allocation-free lookup on hot
/// paths. Cloning a catalog only clones the backing [`Arc`] values.
#[derive(Debug, Clone)]
pub struct GameDataCatalog {
    modules: Arc<[ModuleDefinition]>,
    ship_types: Arc<[ShipTypeDefinition]>,
    module_index: Arc<BTreeMap<ModuleId, ModuleDefinition>>,
    ship_type_index: Arc<BTreeMap<ShipTypeId, ShipTypeDefinition>>,
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
        Ok(Self::from_validated(modules, ship_types))
    }

    /// Validate definitions supplied by an embedding application or fixture.
    ///
    /// This is the in-memory equivalent of [`Self::load_from_paths`]. It still
    /// enforces the complete production ID set; partial catalogs are not valid
    /// Sector construction dependencies.
    pub fn from_definitions(
        modules: Vec<ModuleDefinition>,
        ship_types: Vec<ShipTypeDefinition>,
    ) -> Result<Self, CatalogError> {
        let modules_path = Path::new("<in-memory modules>");
        let ship_types_path = Path::new("<in-memory ship types>");
        module_file::validate_modules(&modules, modules_path)?;
        ship_type_file::validate_ship_types(&ship_types, ship_types_path)?;
        validate_required_ids(&modules, &ship_types, modules_path, ship_types_path)?;
        Ok(Self::from_validated(modules, ship_types))
    }

    fn from_validated(
        mut modules: Vec<ModuleDefinition>,
        mut ship_types: Vec<ShipTypeDefinition>,
    ) -> Self {
        modules.sort_by_key(|definition| definition.id.0);
        ship_types.sort_by_key(|definition| definition.id.0);

        let module_index = modules
            .iter()
            .cloned()
            .map(|definition| (definition.id, definition))
            .collect();
        let ship_type_index = ship_types
            .iter()
            .cloned()
            .map(|definition| (definition.id, definition))
            .collect();

        Self {
            modules: Arc::from(modules),
            ship_types: Arc::from(ship_types),
            module_index: Arc::new(module_index),
            ship_type_index: Arc::new(ship_type_index),
        }
    }

    /// Modules in stable ascending [`ModuleId`] order.
    pub fn modules(&self) -> &[ModuleDefinition] {
        &self.modules
    }

    /// Ship types in stable ascending [`ShipTypeId`] order.
    pub fn ship_types(&self) -> &[ShipTypeDefinition] {
        &self.ship_types
    }

    pub fn module(&self, id: ModuleId) -> Option<&ModuleDefinition> {
        self.module_index.get(&id)
    }

    pub fn ship_type(&self, id: ShipTypeId) -> Option<&ShipTypeDefinition> {
        self.ship_type_index.get(&id)
    }

    pub(crate) fn module_index(&self) -> Arc<BTreeMap<ModuleId, ModuleDefinition>> {
        Arc::clone(&self.module_index)
    }

    pub(crate) fn ship_type_index(&self) -> Arc<BTreeMap<ShipTypeId, ShipTypeDefinition>> {
        Arc::clone(&self.ship_type_index)
    }

    /// Load the required runtime-relative production catalog.
    ///
    /// Paths are resolved only from the process working directory. Production
    /// startup never falls back to files from the source checkout.
    pub fn load_runtime() -> Result<Self, CatalogError> {
        Self::load_production()
    }
}

/// Return the repository catalog used by `dawn-sector` unit tests.
///
/// Production callers must load a catalog explicitly and pass it into Sector
/// construction. This process-local fixture exists only in this crate's test
/// build so unit tests share one complete validated baseline.
#[cfg(test)]
pub(crate) fn test_catalog() -> &'static GameDataCatalog {
    static TEST_CATALOG: OnceLock<GameDataCatalog> = OnceLock::new();
    TEST_CATALOG.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        GameDataCatalog::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog")
    })
}

#[cfg(test)]
pub(crate) fn test_catalog_arc() -> Arc<GameDataCatalog> {
    Arc::new(test_catalog().clone())
}

#[cfg(test)]
pub(crate) fn test_catalog_with_overrides(
    module_overrides: &[ModuleDefinition],
    ship_type_overrides: &[ShipTypeDefinition],
) -> Arc<GameDataCatalog> {
    let mut modules = test_catalog().modules().to_vec();
    for definition in module_overrides {
        match modules.iter_mut().find(|item| item.id == definition.id) {
            Some(existing) => *existing = definition.clone(),
            None => modules.push(definition.clone()),
        }
    }

    let mut ship_types = test_catalog().ship_types().to_vec();
    for definition in ship_type_overrides {
        match ship_types.iter_mut().find(|item| item.id == definition.id) {
            Some(existing) => *existing = definition.clone(),
            None => ship_types.push(definition.clone()),
        }
    }

    Arc::new(
        GameDataCatalog::from_definitions(modules, ship_types)
            .expect("test catalog overrides must remain complete and valid"),
    )
}

#[cfg(test)]
impl GameDataCatalog {
    pub(crate) fn load_test_runtime_directory(root: &Path) -> Result<Self, CatalogError> {
        Self::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
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
