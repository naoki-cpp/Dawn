from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"expected block not found in {path}: {old[:100]!r}")
    file.write_text(text.replace(old, new, 1))


# Compatibility and test fixtures may explicitly use repository TOML, while
# production startup remains bound to cwd-relative packaged data only.
path = Path("crates/dawn-sector/src/game_data/mod.rs")
text = path.read_text()
text = text.replace(
    "static RUNTIME_CATALOG: OnceLock<Result<GameDataCatalog, CatalogError>> = OnceLock::new();\n",
    "static RUNTIME_CATALOG: OnceLock<Result<GameDataCatalog, CatalogError>> = OnceLock::new();\n"
    "static REPOSITORY_CATALOG: OnceLock<Result<GameDataCatalog, CatalogError>> = OnceLock::new();\n",
    1,
)

runtime_fn = '''pub fn runtime_catalog() -> Result<&'static GameDataCatalog, &'static CatalogError> {
    RUNTIME_CATALOG
        .get_or_init(GameDataCatalog::load_production)
        .as_ref()
}
'''
repository_fn = runtime_fn + '''
/// Return the source-checkout catalog for compatibility helpers and test tooling.
///
/// Production startup must use [`runtime_catalog`] instead. Keeping this path
/// explicit prevents missing deployment data from being masked by build inputs.
pub(crate) fn repository_catalog() -> Result<&'static GameDataCatalog, &'static CatalogError> {
    REPOSITORY_CATALOG
        .get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            GameDataCatalog::load_from_paths(
                root.join(PRODUCTION_MODULES_PATH),
                root.join(PRODUCTION_SHIP_TYPES_PATH),
            )
        })
        .as_ref()
}
'''
if runtime_fn not in text:
    raise SystemExit("runtime_catalog function not found")
text = text.replace(runtime_fn, repository_fn, 1)

old_test_impl = '''#[cfg(test)]
impl GameDataCatalog {
    pub(crate) fn load_repository_data() -> Result<Self, CatalogError> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
    }

    pub(crate) fn load_test_runtime_directory(root: &Path) -> Result<Self, CatalogError> {
        Self::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
    }
}
'''
new_test_impl = '''#[cfg(test)]
impl GameDataCatalog {
    pub(crate) fn load_repository_data() -> Result<Self, CatalogError> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
    }

    pub(crate) fn load_test_runtime_directory(root: &Path) -> Result<Self, CatalogError> {
        Self::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
    }
}
'''
if old_test_impl not in text:
    raise SystemExit("test catalog helper block not found")
# Keep the explicit test helpers; this assertion mainly protects script ordering.
text = text.replace(old_test_impl, new_test_impl, 1)
path.write_text(text)

replace_once(
    "crates/dawn-sector/src/modules.rs",
    '''pub fn all_modules() -> Vec<ModuleDefinition> {
    crate::game_data::runtime_catalog()
        .unwrap_or_else(|error| panic!("failed to load authoritative game-data catalog: {error}"))
        .modules()
        .to_vec()
}
''',
    '''pub fn all_modules() -> Vec<ModuleDefinition> {
    crate::game_data::repository_catalog()
        .unwrap_or_else(|error| panic!("failed to load repository game-data catalog: {error}"))
        .modules()
        .to_vec()
}
''',
)

replace_once(
    "crates/dawn-sector/src/ship_types.rs",
    '''pub fn all_ship_types() -> Vec<ShipTypeDefinition> {
    crate::game_data::runtime_catalog()
        .unwrap_or_else(|error| panic!("failed to load authoritative game-data catalog: {error}"))
        .ship_types()
        .to_vec()
}
''',
    '''pub fn all_ship_types() -> Vec<ShipTypeDefinition> {
    crate::game_data::repository_catalog()
        .unwrap_or_else(|error| panic!("failed to load repository game-data catalog: {error}"))
        .ship_types()
        .to_vec()
}
''',
)

# When the entire stat_delta table is omitted, serde must still use the identity
# multiplier rather than the derived f64 default of zero.
replace_once(
    "crates/dawn-sector/src/game_data/module_file.rs",
    '''    #[serde(default)]
    stat_delta: StatDeltaEntry,
''',
    '''    #[serde(default = "default_stat_delta")]
    stat_delta: StatDeltaEntry,
''',
)
replace_once(
    "crates/dawn-sector/src/game_data/module_file.rs",
    '''fn default_speed_multiplier() -> f64 {
    1.0
}
''',
    '''fn default_speed_multiplier() -> f64 {
    1.0
}

fn default_stat_delta() -> StatDeltaEntry {
    StatDeltaEntry {
        speed_multiplier: default_speed_multiplier(),
        ..StatDeltaEntry::default()
    }
}
''',
)
