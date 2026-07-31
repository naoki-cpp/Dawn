from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"expected block not found in {path}: {old[:80]!r}")
    file.write_text(text.replace(old, new, 1))


# Production runtime resolution must never fall back to the build checkout.
path = Path("crates/dawn-sector/src/game_data/mod.rs")
text = path.read_text()
start = text.index("    /// Load the runtime-relative catalog")
end = text.index("\n/// Return the process-wide runtime catalog", start)
replacement = '''    /// Load the required runtime-relative production catalog.
    ///
    /// Paths are resolved only from the process working directory. Production
    /// startup never falls back to files from the source checkout.
    pub fn load_runtime() -> Result<Self, CatalogError> {
        Self::load_production()
    }
}
'''
text = text[:start] + replacement + text[end:]
text = text.replace(
    ".get_or_init(GameDataCatalog::load_runtime)",
    ".get_or_init(GameDataCatalog::load_production)",
    1,
)
old_test_impl = '''#[cfg(test)]
impl GameDataCatalog {
    pub(crate) fn load_repository_data() -> Result<Self, CatalogError> {
        Self::load_runtime()
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
    raise SystemExit("expected test impl not found in game_data/mod.rs")
path.write_text(text.replace(old_test_impl, new_test_impl, 1))

# Custom compatibility paths load and validate only the requested category.
Path("crates/dawn-sector/src/data_loader/mod.rs").write_text(r'''//! Backward-compatible game-data loading entry points.
//!
//! Production paths reach the process-wide strict
//! [`crate::game_data::GameDataCatalog`]. Custom paths retain the historical
//! category-only loading behavior, but validation is strict and fallback values
//! are deliberately ignored.

use crate::game_data::{
    load_modules_file, load_ship_types_file, runtime_catalog, PRODUCTION_MODULES_PATH,
    PRODUCTION_SHIP_TYPES_PATH,
};
use dawn_core::fitting::ModuleDefinition;
use dawn_core::ship_type::ShipTypeDefinition;

/// Load authoritative module definitions.
///
/// The production path returns the module half of the process-wide catalog.
/// A custom path strictly parses and validates that module file by itself.
/// Missing, malformed, or invalid data aborts instead of selecting `fallback`.
pub fn load_modules(path: &str, _fallback: Vec<ModuleDefinition>) -> Vec<ModuleDefinition> {
    if path == PRODUCTION_MODULES_PATH {
        return runtime_catalog()
            .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
            .modules()
            .to_vec();
    }

    load_modules_file(path)
        .unwrap_or_else(|error| panic!("failed to load required module game data: {error}"))
}

/// Load authoritative ship-type definitions.
///
/// The production path returns the ship-type half of the process-wide catalog.
/// A custom path strictly parses and validates that ship-type file by itself.
/// Missing, malformed, or invalid data aborts instead of selecting `fallback`.
pub fn load_ship_types(path: &str, _fallback: Vec<ShipTypeDefinition>) -> Vec<ShipTypeDefinition> {
    if path == PRODUCTION_SHIP_TYPES_PATH {
        return runtime_catalog()
            .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"))
            .ship_types()
            .to_vec();
    }

    load_ship_types_file(path)
        .unwrap_or_else(|error| panic!("failed to load required ship-type game data: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn custom_module_path_loads_one_strict_category() {
        let mut file = tempfile::NamedTempFile::new().expect("temp module file");
        write!(
            file,
            r#"
[[modules]]
id = 999
name = "Custom Sensor"
kind = "Sensor"
slot = "Mid"
activation_mode = "Passive"
"#
        )
        .expect("write module TOML");

        let modules = load_modules(file.path().to_str().expect("UTF-8 path"), Vec::new());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id.0, 999);
    }

    #[test]
    fn custom_ship_type_path_loads_one_strict_category() {
        let mut file = tempfile::NamedTempFile::new().expect("temp ship-type file");
        write!(
            file,
            r#"
[[ship_types]]
id = 999
name = "Custom Frigate"
class = "Frigate"
buildable = false

[ship_types.slot_layout]
high = 1
mid = 1
low = 1
rig = 1

[ship_types.base_stats]
max_speed = 100.0
mass = 1000.0
inertia_modifier = 0.5
max_shield = 10.0
max_armor = 10.0
max_hull = 10.0
lock_time = 1
max_locks = 1
cap_max = 10.0
cap_recharge_per_tick = 1.0
sig_radius = 10.0
"#
        )
        .expect("write ship-type TOML");

        let ship_types = load_ship_types(file.path().to_str().expect("UTF-8 path"), Vec::new());
        assert_eq!(ship_types.len(), 1);
        assert_eq!(ship_types[0].id.0, 999);
    }
}
''')

# Lock in the no-source-checkout-fallback behavior.
tests_path = Path("crates/dawn-sector/src/game_data/tests.rs")
tests = tests_path.read_text()
marker = '''#[test]
fn production_repository_catalog_loads_and_preserves_known_values() {
'''
new_test = '''#[test]
fn missing_runtime_directory_does_not_fallback_to_repository_data() {
    let directory = tempfile::tempdir().expect("temp runtime directory");
    let error = GameDataCatalog::load_test_runtime_directory(directory.path())
        .expect_err("missing packaged data must remain fatal");

    match error {
        CatalogError::Read { path, source, .. } => {
            assert_eq!(path, directory.path().join(PRODUCTION_MODULES_PATH));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected read error, got {other:?}"),
    }
}

#[test]
fn production_repository_catalog_loads_and_preserves_known_values() {
'''
if marker not in tests:
    raise SystemExit("production catalog test marker not found")
tests_path.write_text(tests.replace(marker, new_test, 1))

# Tests use explicit repository paths; production still calls runtime_catalog().
replace_once(
    "crates/dawn-simulation/src/serve/mod.rs",
    '''        node.set_galaxy(std::sync::Arc::new(Galaxy::demo()));
        register_data_driven_definitions(&mut node);
        node
''',
    '''        node.set_galaxy(std::sync::Arc::new(Galaxy::demo()));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = dawn_sector::game_data::GameDataCatalog::load_from_paths(
            root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
            root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog");
        catalog.register_into(&mut node);
        node
''',
)

# Deployment must reject and never omit required catalog files.
deploy = Path("scripts/deploy-pi-cluster.sh")
deploy_text = deploy.read_text()
old_verify = '''\tif [[ ! -f "$repo_root/data/galaxy.toml" ]]; then
\t\techo "Required runtime data file missing: $repo_root/data/galaxy.toml" >&2
\t\texit 1
\tfi
'''
new_verify = '''\tlocal data_file
\tfor data_file in galaxy.toml modules.toml ship_types.toml; do
\t\tif [[ ! -f "$repo_root/data/$data_file" ]]; then
\t\t\techo "Required runtime data file missing: $repo_root/data/$data_file" >&2
\t\t\texit 1
\t\tfi
\tdone
'''
if old_verify not in deploy_text:
    raise SystemExit("deploy verification block not found")
deploy_text = deploy_text.replace(old_verify, new_verify, 1)
old_copy = '''\tcp "$repo_root/data/galaxy.toml" "$stage_dir/data/galaxy.toml"
\tif [[ -f "$repo_root/data/modules.toml" ]]; then
\t\tcp "$repo_root/data/modules.toml" "$stage_dir/data/modules.toml"
\tfi
\tif [[ -f "$repo_root/data/ship_types.toml" ]]; then
\t\tcp "$repo_root/data/ship_types.toml" "$stage_dir/data/ship_types.toml"
\tfi
'''
new_copy = '''\tcp "$repo_root/data/galaxy.toml" "$stage_dir/data/galaxy.toml"
\tcp "$repo_root/data/modules.toml" "$stage_dir/data/modules.toml"
\tcp "$repo_root/data/ship_types.toml" "$stage_dir/data/ship_types.toml"
'''
if old_copy not in deploy_text:
    raise SystemExit("deploy copy block not found")
deploy.write_text(deploy_text.replace(old_copy, new_copy, 1))

# Synchronize normative documentation.
replace_once(
    "docs/architecture/entity-model.md",
    '''**Current implementation:**
- Loaded at startup from `data/ship_types.toml` (DataLoader)
- Falls back to built-in defaults in `ship_types.rs` if the file is absent
- Definitions are immutable; balance changes mean editing TOML + restarting the server (no rebuild)
- `ShipTypeId` is defined in `dawn-core` and included in the `ShipSpawned` event
''',
    '''**Current implementation:**
- Loaded together with module definitions through `GameDataCatalog`
- `data/ship_types.toml` is the only ship-balance authority
- Missing or invalid required game data is a fatal startup error; no Rust fallback exists
- Definitions are immutable; balance changes mean editing TOML + restarting the server (no rebuild)
- `ShipTypeId` is defined in `dawn-core` and included in the `ShipSpawned` event
''',
)

catalog_doc = Path("docs/architecture/game-data-catalog.md")
doc_text = catalog_doc.read_text()
marker = '''Rust retains stable IDs and simulation invariants. Compatibility functions such
as `modules::all_modules()` and `ship_types::all_ship_types()` read the same
repository TOML through `GameDataCatalog`; they do not contain fallback balance
definitions.
'''
replacement = marker + '''
Production startup resolves `data/` only relative to the process working
directory. It never searches the compile-time source checkout. Category-level
compatibility loaders still accept custom paths for tests and tooling, but they
strictly validate only the requested category and never use their legacy
fallback argument.
'''
if marker not in doc_text:
    raise SystemExit("game-data catalog documentation marker not found")
catalog_doc.write_text(doc_text.replace(marker, replacement, 1))
