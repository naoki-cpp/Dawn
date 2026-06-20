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
mod protocol;
mod ws_server;

use dawn_consensus::{RaftActor, RaftActorHandle, RaftActorMessage, RaftState, TcpRaftTransport};
use dawn_core::{DomainEvent, FitModuleCommand, NodeId, SectorBounds, SectorId, ShipId, SlotKind, WarpTarget};
use dawn_event_store::store::EventStore as _;
use dawn_replication::{LogBatch, ReplicationTransport, TcpReplicationTransport};
use dawn_sector::{aoi, galaxy::Galaxy, modules, ship_types, spawner::{generate_ships, SpawnConfig}, transit};
use dawn_sector::node::SimulationNode;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

const AOI_CELL_SIZE: f32 = 30_000.0;
const TICK_MS      : u64 = 100;
const PRODUCTION_GALAXY_PATH: &str = "data/galaxy.toml";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config/node-0.toml");
    let cfg = config::load(config_path)?;

    println!("════════════════════════════════════════════════");
    println!("  dawn-sector-node  node={} sector={}", cfg.node_id, cfg.sector_id);
    println!("  ws={}  raft={}  repl={}", cfg.ws_addr, cfg.raft_addr, cfg.repl_addr);
    println!("  peers: {}", cfg.peers.len());
    println!("════════════════════════════════════════════════");

    let node_id   = NodeId(cfg.node_id);
    let sector_id = SectorId(cfg.sector_id);
    let bounds    = SectorBounds::centered(SectorBounds::DEFAULT_HALF);

    // ── SimulationNode ────────────────────────────────────────────────────────

    let mut node = build_node(&cfg, node_id, sector_id, bounds);
    spawn_npcs(&mut node, cfg.npc_ships);

    // Lookup: SectorId → peer WS address (for client Redirect on jump).
    let peer_ws: HashMap<SectorId, SocketAddr> = cfg.peers.iter()
        .map(|p| (SectorId(p.node_id), p.ws_addr))
        .collect();

    // ── TCP Raft transport ────────────────────────────────────────────────────
    // Pattern: create one (tx, rx) pair.  tx goes to both TcpRaftTransport
    // (for incoming network messages) and RaftActorHandle (for TickElapsed /
    // Propose / GetRole).  rx goes to RaftActor.

    let (actor_tx, actor_rx) = mpsc::unbounded_channel::<RaftActorMessage>();

    let raft_peer_addrs: HashMap<NodeId, SocketAddr> = cfg.peers.iter()
        .map(|p| (NodeId(p.node_id), p.raft_addr))
        .collect();

    let raft_transport = TcpRaftTransport::bind(cfg.raft_addr, raft_peer_addrs, actor_tx.clone())
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind Raft transport on {}: {}", cfg.raft_addr, e))?;

    let peer_ids: Vec<NodeId> = cfg.peers.iter().map(|p| NodeId(p.node_id)).collect();
    let raft_state = RaftState::new_randomized(node_id, peer_ids.clone(), 10, 10, 3, &mut rand::thread_rng());

    let (committed_tx, mut committed_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(
        RaftActor::new(raft_state, peer_ids, Arc::new(raft_transport), actor_rx, committed_tx).run()
    );
    let raft = RaftActorHandle::new(actor_tx);

    // ── TCP Replication transport ─────────────────────────────────────────────

    let repl_transport = TcpReplicationTransport::bind(cfg.repl_addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind replication transport on {}: {}", cfg.repl_addr, e))?;

    for peer in &cfg.peers {
        if let Err(e) = repl_transport.connect_peer(peer.repl_addr).await {
            eprintln!("[Node] warning: could not connect to peer replication at {}: {}", peer.repl_addr, e);
        }
    }
    let mut repl_rx = repl_transport.subscribe();

    // ── WebSocket server ──────────────────────────────────────────────────────

    let server = ws_server::WsServer::bind(cfg.ws_addr).await?;
    let server  = Arc::new(server);

    // Raft warm-up: tick until a leader is elected (≤ 20 ticks election timeout).
    println!("[Node] Raft warm-up (30 ticks)...");
    for _ in 0..30 {
        transit::step_cluster_node(&mut node, &raft, &mut committed_rx, &[]);
    }
    println!("[Node] warm-up done. Waiting for players...");

    // ── Connection-accept channels ────────────────────────────────────────────

    let (new_conn_tx, mut new_conn_rx) =
        mpsc::unbounded_channel::<(tokio::net::TcpStream, SocketAddr)>();
    let (ready_sess_tx, mut ready_sess_rx) =
        mpsc::unbounded_channel::<ws_server::PlayerSession>();

    let server_clone = server.clone();
    tokio::spawn(async move {
        loop {
            if let Some(conn) = server_clone.try_accept_raw().await {
                let _ = new_conn_tx.send(conn);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    // ── Main tick loop ────────────────────────────────────────────────────────

    let mut sessions    : Vec<ws_server::PlayerSession> = Vec::new();
    let mut ship_player : HashMap<ShipId, dawn_core::PlayerId> = HashMap::new();
    let mut prev_visible: HashMap<dawn_core::PlayerId, Vec<ShipId>> = HashMap::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    loop {
        interval.tick().await;
        let tick_started = std::time::Instant::now();

        // Accept new TCP connections → spawn handshake task.
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            if node.at_population_cap() {
                eprintln!("[Node] connection from {addr} refused: at population cap");
                drop(stream);
                continue;
            }
            let player_id     = node.next_player_id();
            let ship_id       = node.spawn_player_ship_at_pub(player_id, dawn_core::Position::ORIGIN);
            let initial_state = node.get_ship_position(ship_id)
                .map(|pos| node.build_initial_state_json_for(pos, AOI_CELL_SIZE))
                .unwrap_or_else(|| node.build_initial_state_json());
            let player_fitting = node.build_player_fitting_json(ship_id);
            ship_player.insert(ship_id, player_id);
            let tx = ready_sess_tx.clone();
            tokio::spawn(async move {
                match ws_server::WsServer::handshake(stream, addr, player_id, ship_id, &initial_state, player_fitting).await {
                    Ok(sess) => { let _ = tx.send(sess); }
                    Err(e)   => eprintln!("[Node] handshake failed: {e}"),
                }
            });
        }

        // Promote completed handshakes to active sessions.
        while let Ok(sess) = ready_sess_rx.try_recv() {
            println!("[Node] {:?} joined with ship #{}", sess.player_id, sess.ship_id.raw());
            let seed = node.get_ship_position(sess.ship_id)
                .map(|pos| node.ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            prev_visible.insert(sess.player_id, seed);
            sessions.push(sess);
        }

        // Drain incoming replication batches from peers.
        while let Ok(batch) = repl_rx.try_recv() {
            // Full anti-entropy apply is 8D-2d scope; log for observability.
            eprintln!("[Node] recv repl batch sector={:?} n={}", batch.sector_id, batch.events.len());
        }

        let event_cursor = node.total_event_count() as u64;

        // Collect player commands.
        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();
        let mut pending_jumps: Vec<(usize, dawn_core::JumpCommand)> = Vec::new();

        for (i, sess) in sessions.iter_mut().enumerate() {
            while let Some(cmd) = sess.try_recv_command() {
                match cmd {
                    dawn_actor::ClientCommand::Move(mv)      => { node.apply_move_command_owned(sess.player_id, mv.ship_id, mv.target_position); }
                    dawn_actor::ClientCommand::LockOn(lo)    => { if node.owns_ship(sess.player_id, lo.ship_id) { lock_commands.push(lo); } }
                    dawn_actor::ClientCommand::Activate(c)   => { node.activate_module_owned(sess.player_id, c); }
                    dawn_actor::ClientCommand::Deactivate(c) => { node.deactivate_module_owned(sess.player_id, c); }
                    dawn_actor::ClientCommand::Attack(_)     => {}
                    dawn_actor::ClientCommand::Stop(s)       => { node.apply_stop_command_owned(sess.player_id, s.ship_id); }
                    dawn_actor::ClientCommand::Approach(a)   => { node.apply_approach_command_owned(sess.player_id, a); }
                    dawn_actor::ClientCommand::Warp(w)       => { node.apply_warp_command_owned(sess.player_id, w); }
                    dawn_actor::ClientCommand::Jump(j)       => { pending_jumps.push((i, j)); break; }
                }
            }
        }

        for (idx, j) in pending_jumps {
            let sess = &sessions[idx];
            let ship_owned = j.ship_id == sess.ship_id;
            let in_range   = ship_owned && node.can_propose_jump(j.ship_id, j.gate_id);
            if in_range {
                let to = node.jump_gate(j.gate_id).expect("gate must exist").to_sector;
                raft.propose(transit::TransitOp::Request { ship_id: j.ship_id, to, gate_id: Some(j.gate_id) }.encode());
                println!("[Node] Jump proposed: ship #{} gate #{}", j.ship_id.raw(), j.gate_id.0);
            } else if ship_owned && node.apply_warp_command(j.ship_id, WarpTarget::Gate(j.gate_id), true) {
                println!("[Node] Jump: ship #{} out of range — auto-warp to gate #{} started", j.ship_id.raw(), j.gate_id.0);
            }
        }

        // Handle auto-jumps triggered by warp completion.
        for (ship_id, gate_id) in node.drain_pending_auto_jumps() {
            if node.can_propose_jump(ship_id, gate_id) {
                let to = node.jump_gate(gate_id).expect("gate must exist").to_sector;
                raft.propose(transit::TransitOp::Request { ship_id, to, gate_id: Some(gate_id) }.encode());
                println!("[Node] Auto-jump proposed: ship #{} gate #{}", ship_id.raw(), gate_id.0);
            }
        }

        // Tick (Raft steps + game logic).
        transit::step_cluster_node(&mut node, &raft, &mut committed_rx, &lock_commands);

        // Broadcast new events to peer replication subscribers (8D-2c gossip).
        let new_events: Vec<DomainEvent> = node.event_store()
            .iter_from(event_cursor)
            .map(|r| r.event.clone())
            .collect();
        if !new_events.is_empty() {
            let batch = LogBatch::new(sector_id, event_cursor, new_events.clone());
            repl_transport.broadcast(batch);
        }

        // Detect ships that jumped out of this node's sector.
        let jumped_ships: HashMap<ShipId, SectorId> = new_events.iter()
            .filter_map(|e| if let DomainEvent::JumpGateUsed(j) = e {
                if j.from_sector == sector_id { Some((j.ship_id, j.to_sector)) } else { None }
            } else { None })
            .collect();

        // AoI delivery and session management.
        let grid = aoi::CellGrid::build(AOI_CELL_SIZE, node.ship_positions());

        sessions.retain_mut(|sess| {
            // Player's ship jumped to another node → Redirect and drop session.
            if let Some(&dest) = jumped_ships.get(&sess.ship_id) {
                if let Some(&ws_addr) = peer_ws.get(&dest) {
                    let msg = protocol::redirect_json(ws_addr);
                    sess.conn.send_raw(&msg);
                    println!("[Node] Redirect {:?} → {ws_addr}", sess.player_id);
                }
                prev_visible.remove(&sess.player_id);
                return false;
            }

            let curr = node.get_ship_position(sess.ship_id)
                .map(|pos| grid.neighbors_of(pos))
                .unwrap_or_default();
            let prev = prev_visible.entry(sess.player_id).or_default();
            deliver_aoi_frame(sess, &node, curr, prev, &new_events)
        });
        prev_visible.retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));

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

fn build_node(cfg: &config::NodeConfig, node_id: NodeId, sector_id: SectorId, bounds: SectorBounds) -> SimulationNode {
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
            slot     : SlotKind::High,
            module_id: modules::MODULE_RAILGUN_SMALL,
        });
    }
}

/// Deliver one AoI frame to a session. Returns `false` when the connection dropped.
fn deliver_aoi_frame(
    sess      : &mut ws_server::PlayerSession,
    node      : &SimulationNode,
    curr      : Vec<ShipId>,
    prev      : &mut Vec<ShipId>,
    new_events: &[DomainEvent],
) -> bool {
    let destroyed_this_tick: std::collections::HashSet<ShipId> = new_events.iter()
        .filter_map(|e| if let DomainEvent::ShipDestroyed(d) = e { Some(d.ship_id) } else { None })
        .collect();

    let old_prev = prev.clone();
    let (entered, left) = aoi::aoi_delta(&old_prev, &curr);
    *prev = curr.clone();

    for &id in entered.iter().filter(|&&id| id != sess.ship_id) {
        if let Some(msg) = node.aoi_enter_json(id) {
            if !sess.conn.send_raw(&msg) { return false; }
        }
    }
    for &id in left.iter().filter(|&&id| id != sess.ship_id && !destroyed_this_tick.contains(&id)) {
        if !sess.conn.send_raw(&aoi::aoi_leave_json(id)) { return false; }
    }

    let visible_events: Vec<_> = new_events.iter()
        .filter(|e| {
            if let DomainEvent::ShipDestroyed(d) = e {
                return old_prev.binary_search(&d.ship_id).is_ok()
                    || old_prev.binary_search(&d.killer_id).is_ok()
                    || aoi::event_visible_to(e, &curr);
            }
            aoi::event_visible_to(e, &curr)
        })
        .cloned()
        .collect();
    sess.send_events(&visible_events)
}
