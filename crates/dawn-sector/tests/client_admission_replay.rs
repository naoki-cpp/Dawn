use dawn_core::{NodeId, Position, SectorBounds, SectorId};
use dawn_sector::{
    client_admission::ClientAdmissionIntent,
    client_admission_resolution::{resolve_client_admission, ClientAdmissionResolution},
    galaxy::Galaxy,
    game_data::{GameDataCatalog, PRODUCTION_MODULES_PATH, PRODUCTION_SHIP_TYPES_PATH},
    node::SimulationNode,
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
fn admission_outputs_are_explicit_and_not_hidden_in_a_node_store() {
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        galaxy,
        catalog,
    );
    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission should begin");
    assert_eq!(node.ship_count(), 0);
    assert!(node.pending_events().iter().any(|event| matches!(
        event,
        dawn_core::DomainEvent::ClientAdmissionIdentityReserved(_)
    )));

    assert!(matches!(
        resolve_client_admission(&mut node, attempt, Err::<(), _>(())),
        ClientAdmissionResolution::Aborted { .. }
    ));
    assert_eq!(node.ship_count(), 0);
}
