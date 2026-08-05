use dawn_core::{NodeId, Position, SectorBounds, SectorId};
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use dawn_sector::{
    client_admission::{ClientAdmissionIntent, ClientAdmissionRefusal},
    client_admission_resolution::{resolve_client_admission, ClientAdmissionResolution},
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
fn in_flight_fresh_admission_keeps_only_a_durable_identity_watermark() {
    let galaxy = Arc::new(Galaxy::demo());
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
    );
    let pre_begin_snapshot = node.take_snapshot();

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission should begin");
    let reserved_player_id = attempt.player_id();
    let reserved_ship_id = attempt.ship_id();
    let snapshot_during_handshake = node.take_snapshot();

    assert_eq!(node.ship_count(), 0);
    assert!(snapshot_during_handshake.ships.is_empty());
    assert!(matches!(
        node.event_store().all_records(),
        [record]
            if matches!(
                &record.event,
                dawn_core::DomainEvent::ClientAdmissionIdentityReserved(event)
                    if event.player_id == reserved_player_id && event.ship_id == reserved_ship_id
            )
    ));

    let mut replay_store = InMemoryEventStore::new();
    for record in node.event_store().all_records() {
        replay_store.append(record.event.clone());
    }
    drop(attempt);

    let catalog = repository_catalog();
    let mut restored = SimulationNode::restore_from(
        replay_store,
        &pre_begin_snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );

    assert_eq!(restored.ship_count(), 0);
    assert!(restored.ship_absolute_pos(reserved_ship_id).is_none());
    let retry = restored
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("restored node may admit a new client");
    assert_ne!(retry.player_id(), reserved_player_id);
    assert_ne!(retry.ship_id(), reserved_ship_id);
    assert!(matches!(
        resolve_client_admission::<_, (), _>(&mut restored, retry, Err(())),
        ClientAdmissionResolution::Aborted { .. }
    ));
}

#[test]
fn committed_fresh_admission_replays_complete_state_and_grants_starter_once() {
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
    );
    for definition in catalog.modules() {
        node.register_module(definition.clone());
    }
    for definition in catalog.ship_types() {
        node.register_ship_type(definition.clone());
    }
    let pre_commit_snapshot = node.take_snapshot();
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
    assert!(matches!(
        resolve_client_admission(&mut node, attempt, Ok::<_, ()>(())),
        ClientAdmissionResolution::Committed { .. }
    ));

    let records = node.event_store().all_records();
    assert_eq!(records.len(), 2);
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(dawn_core::DomainEvent::ClientAdmissionCommitted(event))
            if event.player_id == player_id
                && event.ship_id == ship_id
                && event.fitting.high.len() == 1
                && event.fitting.mid.len() == 2
    ));
    assert!(!records.iter().any(|record| matches!(
        record.event,
        dawn_core::DomainEvent::ShipSpawned(_) | dawn_core::DomainEvent::ShipFitted(_)
    )));

    let mut replay_store = InMemoryEventStore::new();
    for record in records {
        replay_store.append(record.event.clone());
    }
    let mut restored = SimulationNode::restore_from(
        replay_store,
        &pre_commit_snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );
    assert_eq!(restored.ship_count(), 1);
    assert!(restored.ship_absolute_pos(ship_id).is_some());

    let directory = tempfile::tempdir().unwrap();
    let db_path = directory.path().join("station.sqlite");
    let db_path = db_path.to_str().unwrap();
    restored.open_station_inventory_db(db_path).unwrap();
    let starter = dawn_core::ItemId::PackagedShip(dawn_sector::ship_types::SHIP_TYPE_MAGPIE);
    assert_eq!(
        restored.station_item_count(player_id, dawn_core::StationId(0), starter),
        1
    );
    restored.open_station_inventory_db(db_path).unwrap();
    assert_eq!(
        restored.station_item_count(player_id, dawn_core::StationId(0), starter),
        1
    );
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
    let _player_id = dawn_core::PlayerId(7);
    let _ship_id = dawn_core::ShipId::new(NodeId(9), 1);

    let refusal = node
        .begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: dawn_core::ResumeTicket::from_bytes([88; 32]),
            },
            AOI_CELL_SIZE,
        )
        .expect_err("missing resume must be refused");

    assert_eq!(refusal, ClientAdmissionRefusal::ResumeTicketInvalid);
    assert_eq!(node.ship_count(), 0);
    assert!(node.event_store().all_records().is_empty());
}
