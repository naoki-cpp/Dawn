use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId};
use dawn_event_store::FileEventStore;
use dawn_sector::{
    client_admission::{ClientAdmissionIntent, ClientAdmissionRefusal},
    galaxy::Galaxy,
    game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH},
    node::SimulationNode,
    persistence::StateSnapshot,
};
use std::{path::Path, sync::Arc};

const AOI_CELL_SIZE: f64 = 1_000.0;

fn repository_catalog() -> GameDataCatalog {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    GameDataCatalog::load_from_paths(
        root.join(PRODUCTION_MODULES_PATH),
        root.join(PRODUCTION_SHIP_TYPES_PATH),
    )
    .expect("repository game-data catalog")
}

fn register_catalog(node: &mut SimulationNode<FileEventStore>, catalog: &GameDataCatalog) {
    for definition in catalog.modules() {
        node.register_module(definition.clone());
    }
    for definition in catalog.ship_types() {
        node.register_ship_type(definition.clone());
    }
}

#[test]
fn client_visible_fresh_identity_recovers_after_restart_before_commit() {
    let directory = tempfile::tempdir().unwrap();
    let event_path = directory.path().join("events.log");
    let snapshot_path = directory.path().join("snapshot.bin");
    let db_path = directory.path().join("station.sqlite");
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();

    let store = FileEventStore::open(&event_path).unwrap();
    let mut node = SimulationNode::with_store(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
        store,
    );
    register_catalog(&mut node, &catalog);
    node.open_station_inventory_db(db_path.to_str().unwrap())
        .unwrap();
    node.take_snapshot().save(&snapshot_path).unwrap();

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::new(30_000.0, 0.0, 0.0),
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission");
    let player_id = attempt.player_id();
    let ship_id = attempt.ship_id();
    drop(attempt);
    drop(node);

    let snapshot = StateSnapshot::load(&snapshot_path).unwrap();
    let store = FileEventStore::open(&event_path).unwrap();
    let mut restored = SimulationNode::restore_from(
        store,
        &snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );
    restored
        .open_station_inventory_db(db_path.to_str().unwrap())
        .unwrap();
    assert!(restored.ship_absolute_pos(ship_id).is_none());

    let retry = restored
        .begin_client_admission(
            ClientAdmissionIntent::Resume { player_id, ship_id },
            AOI_CELL_SIZE,
        )
        .expect("the exact client-visible identity must reclaim its prepared admission");
    assert!(!retry.is_resumed());
    retry.commit(&mut restored).expect("recovered fresh commit");
    assert!(restored.ship_absolute_pos(ship_id).is_some());
    assert!(restored.apply_stop_command_owned(player_id, ship_id));
}

#[test]
fn ownership_binding_survives_checkpoint_compaction() {
    let directory = tempfile::tempdir().unwrap();
    let event_path = directory.path().join("events.log");
    let snapshot_path = directory.path().join("snapshot.bin");
    let cold_path = directory.path().join("events.cold");
    let db_path = directory.path().join("station.sqlite");
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();

    let store = FileEventStore::open(&event_path).unwrap();
    let mut node = SimulationNode::with_store(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
        store,
    );
    register_catalog(&mut node, &catalog);
    node.open_station_inventory_db(db_path.to_str().unwrap())
        .unwrap();

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission");
    let player_id = attempt.player_id();
    let ship_id = attempt.ship_id();
    attempt.commit(&mut node).expect("fresh commit");
    node.checkpoint(&snapshot_path, &cold_path).unwrap();
    drop(node);

    let snapshot = StateSnapshot::load(&snapshot_path).unwrap();
    let store = FileEventStore::open(&event_path).unwrap();
    let mut restored = SimulationNode::restore_from(
        store,
        &snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );
    restored
        .open_station_inventory_db(db_path.to_str().unwrap())
        .unwrap();

    let attacker = PlayerId(player_id.0 + 100);
    assert_eq!(
        restored
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: attacker,
                    ship_id,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("a different Player cannot claim the restored Ship"),
        ClientAdmissionRefusal::ResumeIdentityConflict {
            player_id: attacker,
            ship_id,
        }
    );

    let exact = restored
        .begin_client_admission(
            ClientAdmissionIntent::Resume { player_id, ship_id },
            AOI_CELL_SIZE,
        )
        .expect("the original identity reconnects after checkpoint");
    exact.abort(&mut restored);
}
