//! `dawn-sector-node` — production binary for one physical Sector node (8D-4).
//!
//! Each instance owns exactly one Sector and connects to its peers via TCP:
//!   - **Raft RPC** (`TcpRaftTransport`, 8D-3) for Sector-Transit consensus (ADR-0014)
//!   - **Log replication gossip** (`TcpReplicationTransport`, 8D-2c) for event-log distribution
//!   - **WebSocket** for Godot client connections
//!
//! Usage:
//! ```text
//! sector-node config/node-0.toml
//! sector-node config/node-1.toml
//! sector-node config/node-2.toml
//! ```

mod config;
mod data_loader;
mod runtime;

use dawn_actor::{protocol::ResumeIdentity, ws_server};
use dawn_consensus::{RaftActor, RaftActorHandle, RaftActorMessage, RaftState, TcpRaftTransport};
use dawn_core::{
    FitModuleCommand, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, SlotKind,
};
use dawn_replication::{Ingest, ReplicaSet, ReplicationTransport, TcpReplicationTransport};
use dawn_sector::node::SimulationNode;
use dawn_sector::{
    galaxy::Galaxy,
    modules, ship_types,
    spawner::{generate_ships, SpawnConfig},
    transit,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

const AOI_CELL_SIZE: f32 = 30_000.0;
const TICK_MS: u64 = 100;
/// Cap on the suffix length an anti-entropy gap request may ask for.
const MAX_REPL_SUFFIX: usize = 4096;
const PRODUCTION_GALAXY_PATH: &str = "data/galaxy.toml";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("config/node-0.toml");
    let cfg = config::load(config_path)?;

    println!("════════════════════════════════════════════════");
    println!(
        "  dawn-sector-node  node={} sector={}",
        cfg.node_id, cfg.sector_id
    );
    println!(
        "  ws={}  raft={}  repl={}",
        cfg.ws_addr, cfg.raft_addr, cfg.repl_addr
    );
    println!("  peers: {}", cfg.peers.len());
    println!("════════════════════════════════════════════════");

    let node_id = NodeId(cfg.node_id);
    let sector_id = SectorId(cfg.sector_id);
    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);

    // ── SimulationNode ────────────────────────────────────────────────────────

    let mut node = build_node(&cfg, node_id, sector_id, bounds);
    spawn_npcs(&mut node, cfg.npc_ships);

    // Lookup: SectorId → peer WS address (for client Redirect on jump).
    let peer_ws: HashMap<SectorId, SocketAddr> = cfg
        .peers
        .iter()
        .map(|p| (SectorId(p.node_id), p.ws_addr))
        .collect();

    // ── TCP Raft transport ────────────────────────────────────────────────────
    // Pattern: create one (tx, rx) pair.  tx goes to both TcpRaftTransport
    // (for incoming network messages) and RaftActorHandle (for TickElapsed /
    // Propose / GetRole).  rx goes to RaftActor.

    let (actor_tx, actor_rx) = mpsc::unbounded_channel::<RaftActorMessage>();

    let raft_peer_addrs: HashMap<NodeId, SocketAddr> = cfg
        .peers
        .iter()
        .map(|p| (NodeId(p.node_id), p.raft_addr))
        .collect();

    let raft_transport = TcpRaftTransport::bind(cfg.raft_addr, raft_peer_addrs, actor_tx.clone())
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to bind Raft transport on {}: {}", cfg.raft_addr, e)
        })?;

    let peer_ids: Vec<NodeId> = cfg.peers.iter().map(|p| NodeId(p.node_id)).collect();
    let raft_state = RaftState::new_randomized(
        node_id,
        peer_ids.clone(),
        10,
        10,
        3,
        &mut rand::thread_rng(),
    );

    let (committed_tx, mut committed_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(
        RaftActor::new(
            raft_state,
            peer_ids,
            Arc::new(raft_transport),
            actor_rx,
            committed_tx,
        )
        .run(),
    );
    let raft = RaftActorHandle::new(actor_tx);

    // ── TCP Replication transport ─────────────────────────────────────────────

    let repl_transport = TcpReplicationTransport::bind(cfg.repl_addr)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to bind replication transport on {}: {}",
                cfg.repl_addr,
                e
            )
        })?;

    for peer in &cfg.peers {
        if let Err(e) = repl_transport.connect_peer(peer.repl_addr).await {
            eprintln!(
                "[Node] warning: could not connect to peer replication at {}: {}",
                peer.repl_addr, e
            );
        }
    }
    let mut repl_rx = repl_transport.subscribe();
    // Consumer side of log-shipping gossip: holds a gap-checked, idempotent
    // replica of each peer Sector's log (ADR-0021). Held but not yet applied
    // to a live world — see the drain loop below.
    let mut replicas = ReplicaSet::new(MAX_REPL_SUFFIX);

    // ── WebSocket server ──────────────────────────────────────────────────────

    let server = ws_server::WsServer::bind(cfg.ws_addr).await?;
    let server = Arc::new(server);

    // Raft warm-up: tick until a leader is elected (≤ 20 ticks election timeout).
    println!("[Node] Raft warm-up (30 ticks)...");
    for _ in 0..30 {
        transit::step_cluster_node(&mut node, &raft, &mut committed_rx, &[]);
    }
    println!("[Node] warm-up done. Waiting for players...");

    // ── Connection-accept channels ────────────────────────────────────────────

    let (handshake_req_tx, mut handshake_req_rx) =
        mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
    let (ready_sess_tx, mut ready_sess_rx) = mpsc::unbounded_channel::<ws_server::PlayerSession>();

    let server_clone = server.clone();
    tokio::spawn(async move {
        loop {
            if let Some((stream, addr)) = server_clone.try_accept_raw().await {
                let tx = handshake_req_tx.clone();
                tokio::spawn(async move {
                    match ws_server::WsServer::accept_handshake_request(stream, addr).await {
                        Ok(req) => {
                            let _ = tx.send(req);
                        }
                        Err(e) => eprintln!("[Node] handshake request failed: {e}"),
                    }
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    // ── Main tick loop ────────────────────────────────────────────────────────

    let mut runtime = runtime::SectorNodeRuntime::new(sector_id, AOI_CELL_SIZE, peer_ws);
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    loop {
        interval.tick().await;
        let tick_started = std::time::Instant::now();

        // Complete handshakes after Hello tells us whether this is a fresh
        // client or a Redirect resume from another physical node.
        while let Ok(request) = handshake_req_rx.try_recv() {
            let handshake_identity = match select_handshake_identity(&mut node, request.resume) {
                HandshakeSelection::Selected(identity) => identity,
                HandshakeSelection::RefusedResumeMissingShip(resume) => {
                    eprintln!(
                        "[Node] resume refused from {}: ship #{} is not in Sector {}",
                        request.peer_addr,
                        resume.ship_id.raw(),
                        sector_id.0
                    );
                    continue;
                }
                HandshakeSelection::RefusedFreshAtCap => {
                    eprintln!(
                        "[Node] connection from {} refused: at population cap",
                        request.peer_addr
                    );
                    continue;
                }
            };
            if handshake_identity.resumed {
                println!(
                    "[Node] resume accepted from {}: {:?} ship #{}",
                    request.peer_addr,
                    handshake_identity.player_id,
                    handshake_identity.ship_id.raw()
                );
            };
            let player_id = handshake_identity.player_id;
            let ship_id = handshake_identity.ship_id;
            let initial_state = node
                .ship_absolute_pos(ship_id)
                .map(|pos| node.build_initial_state_json_for(pos, AOI_CELL_SIZE))
                .unwrap_or_else(|| node.build_initial_state_json());
            let player_fitting = node.build_player_fitting_json(ship_id);
            let tx = ready_sess_tx.clone();
            tokio::spawn(async move {
                match request
                    .complete(player_id, ship_id, &initial_state, player_fitting)
                    .await
                {
                    Ok(sess) => {
                        let _ = tx.send(sess);
                    }
                    Err(e) => eprintln!("[Node] handshake failed: {e}"),
                }
            });
        }

        // Promote completed handshakes to active sessions.
        while let Ok(sess) = ready_sess_rx.try_recv() {
            runtime.promote_ready_session(&node, sess);
        }

        // Drain incoming replication batches from peers into the per-Sector
        // replica (gap-checked, idempotent). This is log-shipping's consumer
        // side (ADR-0021): the replica retains each peer Sector's ordered log.
        // Applying those events to a live world (and failover takeover) is a
        // separate feature — see ReplicaSet docs.
        while let Ok(batch) = repl_rx.try_recv() {
            match replicas.ingest(&batch) {
                Ingest::Applied {
                    sector_id,
                    applied,
                    next_index,
                } => {
                    if applied > 0 {
                        eprintln!(
                            "[Repl] sector={sector_id:?} +{applied} → next_index={next_index}"
                        );
                    }
                }
                Ingest::Duplicate => {} // idempotent drop; nothing to do
                Ingest::Gap(req) => {
                    // The owner's prefix is missing; a future SnapshotTransfer /
                    // anti-entropy request path (ADR-0017) will fill it. Log so
                    // physical-node packet loss shows up during 8D-5.
                    eprintln!(
                        "[Repl] sector={:?} gap: expected from_index={}, awaiting catch-up",
                        req.sector_id, req.from_index
                    );
                }
            }
        }

        runtime.run_frame(&mut node, &raft, &mut committed_rx, &repl_transport);

        // Field observability for 8D-5: a tick that overruns its own period
        // means TCP/WS I/O (Raft, replication, or session delivery) is
        // blocking the simulation loop — the first symptom of WiFi/USB
        // link strain on physical nodes.
        let elapsed = tick_started.elapsed();
        if elapsed.as_millis() as u64 > TICK_MS {
            eprintln!("[Node] tick overrun: {elapsed:?} (budget {TICK_MS}ms)");
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

struct HandshakeIdentity {
    player_id: PlayerId,
    ship_id: ShipId,
    resumed: bool,
}

enum HandshakeSelection {
    Selected(HandshakeIdentity),
    RefusedFreshAtCap,
    RefusedResumeMissingShip(ResumeIdentity),
}

fn select_handshake_identity(
    node: &mut SimulationNode,
    resume: Option<ResumeIdentity>,
) -> HandshakeSelection {
    if let Some(resume) = resume {
        return node
            .adopt_player_ship(resume.ship_id, resume.player_id)
            .then_some(HandshakeSelection::Selected(HandshakeIdentity {
                player_id: resume.player_id,
                ship_id: resume.ship_id,
                resumed: true,
            }))
            .unwrap_or(HandshakeSelection::RefusedResumeMissingShip(resume));
    }

    if node.at_population_cap() {
        return HandshakeSelection::RefusedFreshAtCap;
    }

    let player_id = node.next_player_id();
    // Spawn 2x the Alpha star radius from Sector origin (matches
    // SimulationNode::DEFAULT_PLAYER_SPAWN) -- the star itself sits at
    // the origin, so spawning there put the player inside it.
    let ship_id = node.spawn_player_ship_at_pub(player_id, Position::new(30_000.0, 0.0, 0.0));
    HandshakeSelection::Selected(HandshakeIdentity {
        player_id,
        ship_id,
        resumed: false,
    })
}

fn build_node(
    cfg: &config::NodeConfig,
    node_id: NodeId,
    sector_id: SectorId,
    bounds: SectorBounds,
) -> SimulationNode {
    let mut node = SimulationNode::new(node_id, sector_id, bounds);
    node.set_population_cap(cfg.pop_cap);
    let star_map = load_required_galaxy(PRODUCTION_GALAXY_PATH);
    node.set_galaxy(Arc::new(star_map));
    for def in data_loader::load_modules("data/modules.toml") {
        node.register_module(def);
    }
    for def in data_loader::load_ship_types("data/ship_types.toml") {
        node.register_ship_type(def);
    }
    node
}

fn load_required_galaxy(path: &str) -> Galaxy {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read production galaxy map '{path}': {e}"));
    Galaxy::from_toml_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse production galaxy map '{path}': {e}"))
}

fn spawn_npcs(node: &mut SimulationNode, count: usize) {
    let config = SpawnConfig::default_for_node(NodeId(0));
    for (_, pos, vel) in generate_ships(count, &config, 0) {
        let ship_id = node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: modules::MODULE_RAILGUN_SMALL,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn fresh_handshake_spawns_new_player_ship() {
        let mut node = test_node();

        let selection = select_handshake_identity(&mut node, None);

        let HandshakeSelection::Selected(identity) = selection else {
            panic!("fresh handshake should be accepted below the population cap");
        };
        assert!(!identity.resumed);
        assert_eq!(identity.player_id, PlayerId(0));
        assert_eq!(node.ship_count(), 1);
        assert_eq!(
            node.ship_absolute_pos(identity.ship_id),
            Some([30_000.0, 0.0, 0.0])
        );
    }

    #[test]
    fn resume_handshake_adopts_existing_ship_without_spawning() {
        let mut node = test_node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(
            ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(42.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );

        let selection =
            select_handshake_identity(&mut node, Some(ResumeIdentity { player_id, ship_id }));

        let HandshakeSelection::Selected(identity) = selection else {
            panic!("resume handshake should adopt a ship already in this sector");
        };
        assert!(identity.resumed);
        assert_eq!(identity.player_id, player_id);
        assert_eq!(identity.ship_id, ship_id);
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn resume_handshake_rejects_missing_ship_without_spawning() {
        let mut node = test_node();
        let resume = ResumeIdentity {
            player_id: PlayerId(12),
            ship_id: ShipId::new(NodeId(99), 1),
        };

        let selection = select_handshake_identity(&mut node, Some(resume));

        let HandshakeSelection::RefusedResumeMissingShip(rejected) = selection else {
            panic!("resume handshake should reject a ship absent from this sector");
        };
        assert_eq!(rejected, resume);
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn fresh_handshake_respects_population_cap() {
        let mut node = test_node();
        node.set_population_cap(0);

        let selection = select_handshake_identity(&mut node, None);

        assert!(matches!(selection, HandshakeSelection::RefusedFreshAtCap));
        assert_eq!(node.ship_count(), 0);
    }
}
