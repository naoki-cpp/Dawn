//! Contract test (ADR-0039): proves the real server's `PlayerLoadout` wire
//! payload (`dawn_sector`'s `build_player_loadout_json`) still parses into
//! `dawn_client_core::PlayerLoadoutMsg`. `dawn-sector` is a dev-dependency
//! only -- this does not add a runtime dependency edge, so it doesn't
//! resurrect the previously-rejected `dawn-proto` shared-schema crate.
//!
//! If this test starts failing, the server's wire shape drifted from this
//! crate's types -- update `ModuleRow`/`ItemRow`/`PlayerLoadoutMsg` to match.

use dawn_client_core::PlayerLoadoutMsg;
use dawn_core::{FitModuleCommand, NodeId, Position, SectorBounds, SectorId, SlotKind};
use dawn_sector::node::SimulationNode;

fn test_node() -> SimulationNode {
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
    );
    for def in dawn_sector::modules::all_modules() {
        node.register_module(def);
    }
    for def in dawn_sector::ship_types::all_ship_types() {
        node.register_ship_type(def);
    }
    node
}

#[test]
fn player_loadout_json_from_a_freshly_spawned_ship_parses_into_player_loadout_msg() {
    let mut node = test_node();
    let player_id = node.next_player_id();
    let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

    let json = node
        .build_player_loadout_json(ship_id)
        .expect("freshly spawned ship has a loadout payload");

    let loadout: PlayerLoadoutMsg =
        serde_json::from_str(&json).expect("server PlayerLoadout JSON must parse");

    assert_eq!(loadout.active_ship_id, Some(ship_id.raw()));
    assert!(!loadout.is_docked());
    // A freshly spawned player ship starts with one of every registered
    // module fitted (see spawner_logic.rs's default loadout) plus a starter
    // packaged ship in inventory (9B) -- exercise both row kinds through the
    // real wire shape, not just module rows.
    assert!(!loadout.modules.is_empty());
    assert!(!loadout.inventory.is_empty());
}

#[test]
fn player_loadout_json_with_a_fitted_module_round_trips_stat_delta() {
    let mut node = test_node();
    let player_id = node.next_player_id();
    let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

    let module_id = dawn_sector::modules::all_modules()[0].id;
    let fitted = node.fit_module(FitModuleCommand {
        ship_id,
        slot: SlotKind::High,
        module_id,
    });
    assert!(
        fitted,
        "fitting a registered module onto a known ship must succeed"
    );

    let json = node
        .build_player_loadout_json(ship_id)
        .expect("ship has a loadout payload");
    let loadout: PlayerLoadoutMsg =
        serde_json::from_str(&json).expect("server PlayerLoadout JSON must parse");

    let row = loadout
        .modules
        .iter()
        .find(|m| m.module_id == module_id.0)
        .expect("the module just fitted appears as a row");
    // Just needs to deserialize into a real f64 (i.e. the field existed and
    // had the expected JSON type) -- the specific value is the module
    // registry's concern, not this crate's.
    let _ = row.stat_delta.weapon_range_add;
}
