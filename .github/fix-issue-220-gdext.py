from pathlib import Path

path = Path("crates/dawn-client-gdext/src/loadout_gd.rs")
text = path.read_text()
old = '''        for def in dawn_sector::modules::all_modules() {
            node.register_module(def);
        }
        for def in dawn_sector::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
'''
new = '''        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = dawn_sector::game_data::GameDataCatalog::load_from_paths(
            root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
            root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog");
        catalog.register_into(&mut node);
'''
if old not in text:
    raise SystemExit("gdext test catalog block not found")
path.write_text(text.replace(old, new, 1))
