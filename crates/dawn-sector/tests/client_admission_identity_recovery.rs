use dawn_core::{NodeId, Position, SectorBounds, SectorId};
use dawn_sector::{
    client_admission::ClientAdmissionIntent,
    client_admission_resolution::{resolve_client_admission, ClientAdmissionResolution},
    galaxy::Galaxy,
    game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH},
    node::SimulationNode,
    persistence::StateSnapshot,
};
use std::{path::Path, sync::Arc};

const AOI_CELL_SIZE: f64 = 1_000.0;

fn repository_catalog() -> Arc<GameDataCatalog> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Arc::new(
        GameDataCatalog::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog"),
    )
}

#[test]
fn client_visible_fresh_identity_recovers_after_disconnect_before_commit() {
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
        Arc::clone(&catalog),
    );

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
    let resume_ticket = attempt.resume_ticket();
    assert!(node.ship_absolute_pos(ship_id).is_none());
    assert!(node.pending_events().iter().any(|event| matches!(
        event,
        dawn_core::DomainEvent::ClientAdmissionIdentityReserved(_)
    )));

    // The original handoff attempt must release its live claim before the
    // client can retry the same prepared identity. Concurrent duplicate
    // claims are rejected by the admission boundary.
    assert!(matches!(
        resolve_client_admission(&mut node, attempt, Err::<(), _>("client disconnected")),
        ClientAdmissionResolution::Aborted { .. }
    ));

    let retry = node
        .begin_client_admission(
            ClientAdmissionIntent::Resume { resume_ticket },
            AOI_CELL_SIZE,
        )
        .expect("the prepared identity remains claimable after disconnect");
    assert!(!retry.is_resumed());
    assert!(matches!(
        resolve_client_admission(&mut node, retry, Ok::<_, ()>(())),
        ClientAdmissionResolution::Committed { .. }
    ));
    assert!(node.ship_absolute_pos(ship_id).is_some());
    assert!(node.apply_stop_command_owned(player_id, ship_id));
}

#[test]
fn ownership_binding_survives_checkpoint_compaction() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot_path = directory.path().join("snapshot.bin");
    let cold_path = directory.path().join("events.cold");
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();

    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
        Arc::clone(&catalog),
    );

    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission");
    let resume_ticket = attempt.resume_ticket();
    assert!(matches!(
        resolve_client_admission(&mut node, attempt, Ok::<_, ()>(())),
        ClientAdmissionResolution::Committed { .. }
    ));
    let snapshot = node.take_snapshot_at(0);
    snapshot.save(&snapshot_path).unwrap();
    assert!(snapshot_path.exists());
    assert!(!cold_path.exists());

    let snapshot = StateSnapshot::load(&snapshot_path).unwrap();
    let mut restored = SimulationNode::restore_from(&snapshot, galaxy, catalog);
    let exact = restored
        .begin_client_admission(
            ClientAdmissionIntent::Resume { resume_ticket },
            AOI_CELL_SIZE,
        )
        .expect_err("prepared identity is intentionally runtime-owned, not snapshot-owned");
    assert!(matches!(
        exact,
        dawn_sector::client_admission::ClientAdmissionRefusal::ResumeTicketInvalid
    ));
}
