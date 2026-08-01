use super::*;
use std::io::Write;

const VALID_MODULE: &str = r#"
[[modules]]
id = 1
name = "Test Railgun"
kind = "Weapon"
slot = "High"
activation_mode = "Active"
cap_cost_per_cycle = 10.0
cycle_time_ticks = 5

[modules.stat_delta]
weapon_damage_add = 1.0
weapon_range_add = 100.0
"#;

const VALID_SHIP_TYPE: &str = r#"
[[ship_types]]
id = 1
name = "Test Frigate"
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
"#;

fn write_temp(source: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    write!(file, "{source}").expect("write temp TOML");
    file
}

#[test]
fn duplicate_module_ids_are_rejected() {
    let source = format!("{VALID_MODULE}\n{VALID_MODULE}");
    let file = write_temp(&source);
    let error = load_modules_file(file.path()).expect_err("duplicate ids must fail");
    assert!(error.to_string().contains("duplicate module id 1"));
}

#[test]
fn duplicate_ship_type_ids_are_rejected() {
    let source = format!("{VALID_SHIP_TYPE}\n{VALID_SHIP_TYPE}");
    let file = write_temp(&source);
    let error = load_ship_types_file(file.path()).expect_err("duplicate ids must fail");
    assert!(error.to_string().contains("duplicate ship type id 1"));
}

#[test]
fn unknown_module_enum_value_is_rejected() {
    let source = VALID_MODULE.replace("kind = \"Weapon\"", "kind = \"Mystery\"");
    let file = write_temp(&source);
    let error = load_modules_file(file.path()).expect_err("unknown enum must fail");
    assert!(error.to_string().contains("Mystery"));
}

#[test]
fn unknown_ship_class_is_rejected() {
    let source = VALID_SHIP_TYPE.replace("class = \"Frigate\"", "class = \"Carrier\"");
    let file = write_temp(&source);
    let error = load_ship_types_file(file.path()).expect_err("unknown enum must fail");
    assert!(error.to_string().contains("Carrier"));
}

#[test]
fn active_module_with_zero_cycle_is_rejected() {
    let source = VALID_MODULE.replace("cycle_time_ticks = 5", "cycle_time_ticks = 0");
    let file = write_temp(&source);
    let error = load_modules_file(file.path()).expect_err("invalid cycle must fail");
    assert!(error.to_string().contains("cycle_time_ticks is 0"));
}

#[test]
fn negative_ship_mass_is_rejected() {
    let source = VALID_SHIP_TYPE.replace("mass = 1000.0", "mass = -1.0");
    let file = write_temp(&source);
    let error = load_ship_types_file(file.path()).expect_err("invalid mass must fail");
    assert!(error.to_string().contains("mass value -1"));
}

#[test]
fn missing_required_ship_type_file_is_reported() {
    let modules = write_temp(VALID_MODULE);
    let directory = tempfile::tempdir().expect("temp directory");
    let missing_ship_types = directory.path().join("missing-ship-types.toml");

    let error = GameDataCatalog::load_from_paths(modules.path(), &missing_ship_types)
        .expect_err("missing required data must fail");
    match error {
        CatalogError::Read { path, source, .. } => {
            assert_eq!(path, missing_ship_types);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected read error, got {other:?}"),
    }
}

#[test]
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
fn repository_test_fixture_loads_and_preserves_known_values() {
    let catalog = test_catalog();
    assert_eq!(
        catalog.modules.len(),
        crate::modules::REQUIRED_MODULE_IDS.len()
    );
    assert_eq!(
        catalog.ship_types.len(),
        crate::ship_types::REQUIRED_SHIP_TYPE_IDS.len()
    );

    let magpie = catalog
        .ship_types
        .iter()
        .find(|definition| definition.id == crate::ship_types::SHIP_TYPE_MAGPIE)
        .expect("Magpie");
    assert!(magpie.buildable);
    assert_eq!(magpie.base_stats.mass, 12_000_000.0);

    let railgun = catalog
        .modules
        .iter()
        .find(|definition| definition.id == crate::modules::MODULE_RAILGUN_SMALL)
        .expect("Small Railgun");
    assert_eq!(railgun.stat_delta.weapon_damage_add, 25.0);
}
