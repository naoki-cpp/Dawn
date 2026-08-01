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
use dawn_replication::{
    CatchUpConfig, CatchUpEvent, CatchUpFailureKind, CatchUpManager, CatchUpStep, CatchUpTransport,
    CatchUpUnavailable, ReplicaSnapshot, ReplicationTransport, TcpReplicationTransport,
};
use dawn_sector::node::SimulationNode;
use dawn_sector::persistence::{CheckpointConfig, CheckpointScheduler, StateSnapshot};
use dawn_sector::{galaxy::Galaxy, game_data::runtime_catalog, transit};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

const AOI_CELL_SIZE: f64 = 30_000.0;
const TICK_MS: u64 = 100;
/// Cap on the suffix length an anti-entropy gap request may ask for.
const MAX_REPL_SUFFIX: usize = 4096;
const CATCH_UP_RETRY_TICKS: u32 = 10;
const CATCH_UP_MAX_RETRIES: u32 = 5;
const CATCH_UP_MAX_REQUESTS: u32 = 1024;
const CATCH_UP_MAX_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
/// Prevent a peer flood from monopolising one simulation tick.
const MAX_CATCH_UP_MESSAGES_PER_TICK: usize = 32;
/// Retry transient terminal failures after 30 seconds. A later gossip batch
/// observes the cleared failure cursor and starts a fresh bounded session.
const CATCH_UP_FAILURE_RETRY_TICKS: u32 = 300;
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
                cfg.repl_addr,
                e
            )
        })?;

    for peer in &cfg.peers {
        if let Err(e) = repl_transport
            .connect_peer(SectorId(peer.node_id), peer.repl_addr)
            .await
        {
            eprintln!(
                "[Node] warning: could not connect to peer replication at {}: {}",
                peer.repl_addr, e
            );
        }
    }
    let mut repl_rx = repl_transport.subscribe();
    let mut catch_up_rx = repl_transport.subscribe_catch_up();
    // The manager owns all foreign recovery state. It never applies foreign
    // events or snapshots to this node's live SimulationNode world.
    let mut catch_up = CatchUpManager::new(
        sector_id,
        CatchUpConfig {
            max_events: MAX_REPL_SUFFIX,
            retry_interval_ticks: CATCH_UP_RETRY_TICKS,
            max_retries: CATCH_UP_MAX_RETRIES,
            max_requests_per_session: CATCH_UP_MAX_REQUESTS,
            max_snapshot_bytes: CATCH_UP_MAX_SNAPSHOT_BYTES,
        },
    );
    let mut catch_up_failure_retries = HashMap::new();
    // Load and validate the durable snapshot once. Requests clone only the
    // Arc-backed payload; they never synchronously re-read the file in the tick
    // loop.
    let mut replica_snapshot =
        match load_replica_snapshot(&cfg.snapshot_path, sector_id, CATCH_UP_MAX_SNAPSHOT_BYTES) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("[Repl] snapshot fallback unavailable at startup: {error}");
                None
            }
        };

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
        advance_catch_up_failure_retries(&mut catch_up, &mut catch_up_failure_retries);

        admission.advance_handshakes(&mut node, sector_id, AOI_CELL_SIZE);

        // Promote completed handshakes to active sessions.
        while let Some(sess) = admission.try_recv_ready_session() {
            runtime.promote_ready_session(&node, sess);
        }

        // Ordinary gossip is ingested into foreign recovery replicas. A gap
        // immediately produces a bounded directed suffix request.
        while let Ok(batch) = repl_rx.try_recv() {
            emit_catch_up_step(
                &repl_transport,
                &mut catch_up_failure_retries,
                catch_up.ingest_batch(batch),
            );
        }

        // Bound owner/requester catch-up work per simulation tick. The cached
        // Arc-backed snapshot makes retries cheap and avoids synchronous disk
        // I/O on this path.
        for _ in 0..MAX_CATCH_UP_MESSAGES_PER_TICK {
            let Ok(message) = catch_up_rx.try_recv() else {
                break;
            };
            let step =
                catch_up.handle_message(message, node.event_store(), || replica_snapshot.clone());
            emit_catch_up_step(&repl_transport, &mut catch_up_failure_retries, step);
        }
        emit_catch_up_step(
            &repl_transport,
            &mut catch_up_failure_retries,
            catch_up.tick(),
        );

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
                match replica_snapshot_from_state(&snapshot, CATCH_UP_MAX_SNAPSHOT_BYTES) {
                    Ok(cached) => replica_snapshot = Some(cached),
                    Err(error) => {
                        replica_snapshot = None;
                        eprintln!("[Repl] new snapshot cannot be served for catch-up: {error}");
                    }
                }
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

fn emit_catch_up_step<T: CatchUpTransport>(
    transport: &T,
    failure_retries: &mut HashMap<SectorId, u32>,
    step: CatchUpStep,
) {
    for message in step.outbound {
        transport.send_catch_up(message);
    }
    for event in step.events {
        match event {
            CatchUpEvent::Applied {
                sector_id,
                applied,
                next_index,
            } => {
                failure_retries.remove(&sector_id);
                eprintln!(
                    "[Repl] sector={sector_id:?} +{applied} -> next_index={next_index}"
                );
            }
            CatchUpEvent::RequestIssued {
                owner_sector_id,
                from_index,
                request_id,
                attempt,
            } => eprintln!(
                "[Repl] catch-up request owner={owner_sector_id:?} from={from_index} id={request_id} attempt={attempt}"
            ),
            CatchUpEvent::SnapshotInstalled {
                sector_id,
                log_index,
                bytes,
            } => {
                failure_retries.remove(&sector_id);
                eprintln!(
                    "[Repl] sector={sector_id:?} installed recovery snapshot log_index={log_index} bytes={bytes}"
                );
            }
            CatchUpEvent::Completed {
                sector_id,
                next_index,
            } => {
                failure_retries.remove(&sector_id);
                eprintln!(
                    "[Repl] sector={sector_id:?} catch-up complete next_index={next_index}"
                );
            }
            CatchUpEvent::Failed(failure) => {
                if is_transient_catch_up_failure(failure.kind) {
                    failure_retries.insert(failure.sector_id, CATCH_UP_FAILURE_RETRY_TICKS);
                }
                eprintln!("[Repl] catch-up failed: {failure:?}")
            }
        }
    }
}

fn is_transient_catch_up_failure(kind: CatchUpFailureKind) -> bool {
    matches!(
        kind,
        CatchUpFailureKind::RetryExhausted
            | CatchUpFailureKind::Remote(CatchUpUnavailable::SnapshotUnavailable)
            | CatchUpFailureKind::Remote(CatchUpUnavailable::RetainedSuffixUnavailable)
    )
}

fn advance_catch_up_failure_retries(
    catch_up: &mut CatchUpManager,
    failure_retries: &mut HashMap<SectorId, u32>,
) {
    let sectors: Vec<_> = failure_retries.keys().copied().collect();
    for sector_id in sectors {
        let Some(remaining) = failure_retries.get_mut(&sector_id) else {
            continue;
        };
        if *remaining > 1 {
            *remaining -= 1;
            continue;
        }
        failure_retries.remove(&sector_id);
        catch_up.reset_failure(sector_id);
        eprintln!(
            "[Repl] sector={sector_id:?} transient failure cooldown expired; next gossip may restart catch-up"
        );
    }
}

fn load_replica_snapshot(
    path: impl AsRef<Path>,
    expected_sector_id: SectorId,
    max_bytes: usize,
) -> anyhow::Result<Option<ReplicaSnapshot>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let file_len = usize::try_from(std::fs::metadata(path)?.len())
        .map_err(|_| anyhow::anyhow!("snapshot length does not fit usize"))?;
    if file_len > max_bytes {
        anyhow::bail!("snapshot is {file_len} bytes, limit is {max_bytes}");
    }

    let bytes = std::fs::read(path)?;
    let snapshot: StateSnapshot = postcard::from_bytes(&bytes)
        .map_err(|error| anyhow::anyhow!("cannot decode current snapshot: {error}"))?;
    if snapshot.sector_id != expected_sector_id {
        anyhow::bail!(
            "snapshot sector {:?} does not match local sector {:?}",
            snapshot.sector_id,
            expected_sector_id
        );
    }
    Ok(Some(ReplicaSnapshot::new(
        snapshot.sector_id,
        snapshot.log_index,
        bytes,
    )))
}

fn replica_snapshot_from_state(
    snapshot: &StateSnapshot,
    max_bytes: usize,
) -> anyhow::Result<ReplicaSnapshot> {
    let bytes = postcard::to_stdvec(snapshot)
        .map_err(|error| anyhow::anyhow!("cannot encode current snapshot: {error}"))?;
    if bytes.len() > max_bytes {
        anyhow::bail!("snapshot is {} bytes, limit is {max_bytes}", bytes.len());
    }
    Ok(ReplicaSnapshot::new(
        snapshot.sector_id,
        snapshot.log_index,
        bytes,
    ))
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
    let catalog = runtime_catalog()
        .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}"));

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
                SimulationNode::restore_from(
                    store,
                    &snapshot,
                    catalog.modules(),
                    catalog.ship_types(),
                ),
                false,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "[Node] no snapshot at '{}', starting fresh",
                cfg.snapshot_path
            );
            let mut node = SimulationNode::with_store(node_id, sector_id, bounds, store);
            catalog.register_into(&mut node);
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
