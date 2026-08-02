use dawn_core::{NodeId, Position, SectorBounds, SectorId};
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use dawn_sector::{
    client_admission::{ClientAdmissionIntent, ClientAdmissionRefusal},
    galaxy::Galaxy,
    game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH},
    node::SimulationNode,
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

#[test]
fn aborted_fresh_admission_does_not_reappear_after_event_replay() {
    let galaxy = Arc::new(Galaxy::demo());
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
    );
    let empty_snapshot = node.take_snapshot();

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission should begin");
    let ship_id = attempt.ship_id();

    attempt.abort(&mut node);

    assert_eq!(node.ship_count(), 0);
    let mut replay_store = InMemoryEventStore::new();
    for record in node.event_store().all_records() {
        replay_store.append(record.event.clone());
    }

    let catalog = repository_catalog();
    let restored = SimulationNode::restore_from(
        replay_store,
        &empty_snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );

    assert_eq!(restored.ship_count(), 0);
    assert!(restored.ship_absolute_pos(ship_id).is_none());
}

#[test]
fn missing_resume_still_refuses_without_creating_replayable_state() {
    let galaxy = Arc::new(Galaxy::demo());
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        galaxy,
    );
    let player_id = dawn_core::PlayerId(7);
    let ship_id = dawn_core::ShipId::new(NodeId(9), 1);

    let refusal = node
        .begin_client_admission(
            ClientAdmissionIntent::Resume { player_id, ship_id },
            AOI_CELL_SIZE,
        )
        .expect_err("missing resume must be refused");

    assert_eq!(
        refusal,
        ClientAdmissionRefusal::ResumeShipMissing { player_id, ship_id }
    );
    assert_eq!(node.ship_count(), 0);
    assert!(node.event_store().all_records().is_empty());
}
