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

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

mod client_admission;
mod config;
mod runtime;

use dawn_actor::ws_server;
use dawn_consensus::{RaftActor, RaftActorHandle, RaftActorMessage, RaftState, TcpRaftTransport};
use dawn_core::{NodeId, SectorBounds, SectorId};
use dawn_event_store::FileEventStore;
use dawn_replication::{Ingest, ReplicaSet, ReplicationTransport, TcpReplicationTransport};
use dawn_sector::node::SimulationNode;
use dawn_sector::persistence::{CheckpointConfig, CheckpointScheduler, StateSnapshot};
use dawn_sector::{data_loader, galaxy::Galaxy, modules, ship_types, transit};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

const AOI_CELL_SIZE: f64 = 30_000.0;
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

    let (mut node, is_fresh) = build_node(&cfg, node_id, sector_id, bounds);
    if is_fresh {
        node.spawn_npc_frigates(cfg.npc_ships);
    }

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
                cfg.repl_addr, e
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
        let _ = transit::run_runtime_tick(&mut node, &raft, &mut committed_rx, &[], |_, _, _| {});
    }
    println!("[Node] warm-up done. Waiting for players...");

    // ── Client admission ──────────────────────────────────────────────────────

    let mut admission = client_admission::ClientAdmission::start(server.clone());

    // ── Main tick loop ────────────────────────────────────────────────────────

    let mut runtime = runtime::SectorNodeRuntime::new(
        sector_id,
        AOI_CELL_SIZE,
        peer_ws,
        repl_transport.clone(),
        node.event_store(),
    );
    let mut checkpoints = CheckpointScheduler::new(CheckpointConfig {
        interval_ticks: cfg.checkpoint_interval_ticks,
        snapshot_path: cfg.snapshot_path.clone().into(),
        cold_path: cfg.cold_path.clone().into(),
    });
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    loop {
        interval.tick().await;
        let tick_started = std::time::Instant::now();

        admission.advance_handshakes(&mut node, sector_id, AOI_CELL_SIZE);

        // Promote completed handshakes to active sessions.
        while let Some(sess) = admission.try_recv_ready_session() {
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

        runtime.run_frame(&mut node, &raft, &mut committed_rx);

        // Snapshot + compact on a fixed logical-tick cadence (ADR-0017 §5-C).
        // A checkpoint failure (e.g. disk full) is logged and skipped rather
        // than killing the live server -- the hot log keeps appending
        // normally on the next tick either way, and the next scheduled
        // checkpoint will retry.
        match checkpoints.maybe_checkpoint(&mut node) {
            Ok(Some(snapshot)) => {
                println!(
                    "[Node] checkpoint at tick {} (log_index={})",
                    snapshot.tick.value(),
                    snapshot.log_index
                );
            }
            Ok(None) => {}
            Err(e) => eprintln!("[Node] checkpoint failed, will retry next interval: {e}"),
        }

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

/// Builds the node, restoring from `cfg.snapshot_path` if one exists.
///
/// Returns whether the node started fresh (`true`) or was restored from a
/// snapshot (`false`) -- callers must not re-run one-time genesis setup
/// (e.g. `spawn_npc_frigates`) on a restored node, since its NPCs already exist (or
/// were destroyed) in the restored state.
fn build_node(
    cfg: &config::NodeConfig,
    node_id: NodeId,
    sector_id: SectorId,
    bounds: SectorBounds,
) -> (SimulationNode<FileEventStore>, bool) {
    let modules = data_loader::load_modules("data/modules.toml", modules::all_modules());
    let ship_types =
        data_loader::load_ship_types("data/ship_types.toml", ship_types::all_ship_types());

    // FileEventStore::open does not create its parent directory, and a fresh
    // deployment has no `data/node-N/` yet -- create it (and the snapshot/
    // cold-archive/station-inventory-db parents, which are normally the same
    // directory) up front.
    for path in [
        &cfg.event_log_path,
        &cfg.snapshot_path,
        &cfg.cold_path,
        &cfg.station_inventory_db_path,
    ] {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                panic!("failed to create directory '{}': {e}", parent.display())
            });
        }
    }

    let store = FileEventStore::open(&cfg.event_log_path)
        .unwrap_or_else(|e| panic!("failed to open event log '{}': {e}", cfg.event_log_path));

    let (mut node, is_fresh) = match StateSnapshot::load(&cfg.snapshot_path) {
        Ok(snapshot) => {
            println!(
                "[Node] restoring from snapshot (tick={}, log_index={})",
                snapshot.tick.value(),
                snapshot.log_index
            );
            (
                SimulationNode::restore_from(store, &snapshot, &modules, &ship_types),
                false,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "[Node] no snapshot at '{}', starting fresh",
                cfg.snapshot_path
            );
            let mut node = SimulationNode::with_store(node_id, sector_id, bounds, store);
            for def in &modules {
                node.register_module(def.clone());
            }
            for def in &ship_types {
                node.register_ship_type(def.clone());
            }
            (node, true)
        }
        Err(e) => panic!(
            "snapshot '{}' exists but could not be read: {e}",
            cfg.snapshot_path
        ),
    };

    node.set_population_cap(cfg.pop_cap);
    let star_map = Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)
        .unwrap_or_else(|e| panic!("failed to load production galaxy map: {e}"));
    node.set_galaxy(Arc::new(star_map));
    // ADR-0038: Station inventory's durability is independent of the event
    // log / snapshot lifecycle above -- opening it is just pointing at the
    // (persistent, on-disk) file, whether this node is fresh or restored.
    node.open_station_inventory_db(&cfg.station_inventory_db_path)
        .unwrap_or_else(|e| {
            panic!(
                "failed to open station inventory db '{}': {e}",
                cfg.station_inventory_db_path
            )
        });
    (node, is_fresh)
}
