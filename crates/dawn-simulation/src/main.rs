//! Simulation entry point.
//!
//! Phase 1: single-node benchmark  (10,000 ships × 100 ticks)
//! Phase 2: multi-node demo        (3 nodes × 1,000 ships × 20 ticks)
//!
//! Usage:
//!   cargo run -p dawn-simulation --bin simulate
//!   cargo run -p dawn-simulation --bin simulate --release

// Wired into ws_server in 8C-3/8C-4 (ADR-0019); unused until then.
#[allow(dead_code)]
mod aoi;
mod checkpoint;
mod cluster;
mod data_loader;
mod modules;
mod node;
mod ship_types;
mod sector_simulator_actor;
mod snapshot;
mod spawner;
mod star_map;
mod transit;
mod ws_server;

use dawn_actor::ClientCommand;
use dawn_core::{NodeId, SectorBounds, SectorId, ShipId};
use tokio::sync::mpsc;
use node::SimulationNode;
use spawner::{generate_ships, SpawnConfig};
use std::collections::HashMap;
use std::time::Instant;

use cluster::MultiNodeCluster;
use dawn_core::Position;
use dawn_event_store::FileEventStore;
use snapshot::StateSnapshot;
use ws_server::WsServer;

// ── Constants ─────────────────────────────────────────────────────────────────

const P1_SHIPS : usize = 10_000;
const P1_TICKS : usize = 100;

const P2_NODES : usize = 3;
const P2_SHIPS : usize = 1_000;   // per node
const P2_TICKS : usize = 20;

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Phase 4 mode: with --serve, start the Godot-facing WebSocket server
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--serve".to_string()) {
        // --duel: 1 human vs 1 Bot, no NPC ships
        let duel_mode = args.contains(&"--duel".to_string());
        // --ships N sets the NPC count (ignored in --duel mode)
        let ship_count = if duel_mode { 0 } else {
            args.windows(2)
                .find(|w| w[0] == "--ships")
                .and_then(|w| w[1].parse::<usize>().ok())
                .unwrap_or(P4_SHIPS_DEFAULT)
        };
        // --cluster: 3-node Raft cluster so Jump Gates work (ADR-0009)
        if args.contains(&"--cluster".to_string()) {
            run_cluster_server(ship_count).await;
            return;
        }
        run_phase4_server(ship_count, duel_mode).await;
        return;
    }

    // Phase 7 mode: --raft-demo runs an observable 3-node Raft Transit demo
    // (leader election, Transit through the Raft Log, leader failover).
    if args.contains(&"--raft-demo".to_string()) {
        run_raft_demo().await;
        return;
    }

    run_phase1_benchmark();
    println!();
    run_phase2_demo().await;
    println!();
    run_phase3_demo();
}

// ── Phase 7: Raft Transit demo ───────────────────────────────────────────────

/// Observable walkthrough of the Phase 7 pipeline (ADR-0014):
/// leader election → Transit through the Raft Log → leader failover →
/// Transit completing under a partitioned old leader → partition heal.
async fn run_raft_demo() {
    use cluster::MultiNodeCluster;
    use dawn_consensus::Role;
    use dawn_core::{NodeId, Position, SectorId, Velocity};

    const NODES: usize = 3;

    fn print_roles(label: &str, roles: &[(Role, dawn_consensus::Term)]) {
        print!("  [{label}] roles:");
        for (i, (role, term)) in roles.iter().enumerate() {
            print!("  node{i}={role:?}(t{})", term.0);
        }
        println!();
    }

    async fn print_stats(cluster: &MultiNodeCluster) {
        let stats = cluster.get_all_stats().await;
        print!("  ships per sector:");
        for (i, s) in stats.iter().enumerate() {
            print!("  S{i}={}", s.ship_count);
        }
        println!();
    }

    async fn tick_n(cluster: &MultiNodeCluster, n: usize) {
        for _ in 0..n {
            cluster.tick_all().await;
        }
    }

    async fn leader_index(cluster: &MultiNodeCluster, exclude: Option<usize>) -> Option<usize> {
        cluster.raft_roles().await.iter().enumerate()
            .find(|&(i, (role, _))| *role == Role::Leader && Some(i) != exclude)
            .map(|(i, _)| i)
    }

    println!("═══════════════════════════════════════════");
    println!("  Phase 7 — Raft Transit demo (ADR-0014)   ");
    println!("═══════════════════════════════════════════");

    let cluster = MultiNodeCluster::new(NODES);

    // Act 1: leader election.
    println!("\n── Act 1: leader election ──");
    tick_n(&cluster, 30).await;
    let roles = cluster.raft_roles().await;
    print_roles("after 30 ticks", &roles);
    let leader = leader_index(&cluster, None).await.expect("a leader must be elected");
    println!("  → node{leader} is the Leader");

    // Act 2: a Transit through the Raft Log.
    println!("\n── Act 2: Sector Transit through the Raft Log ──");
    let ship = cluster.nodes()[0]
        .spawn_ship(Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0))
        .await;
    println!("  spawned {ship:?} in Sector 0");
    print_stats(&cluster).await;
    let accepted = cluster.nodes()[0].transit(ship, SectorId(1)).await;
    println!("  TransitCommand(S0 → S1) accepted: {accepted}");
    tick_n(&cluster, 30).await;
    println!("  after 30 ticks (Request + Commit rounds on the Raft Log):");
    print_stats(&cluster).await;

    // Act 3: partition the leader; the survivors elect a new one.
    println!("\n── Act 3: leader failure ──");
    println!("  partitioning node{leader} (current Leader) ...");
    cluster.partition_node(NodeId(leader as u8));
    tick_n(&cluster, 30).await;
    let roles = cluster.raft_roles().await;
    print_roles("after 30 ticks", &roles);
    let new_leader = leader_index(&cluster, Some(leader)).await
        .expect("survivors must elect a new leader");
    println!("  → node{new_leader} is the new Leader (node{leader} is isolated)");

    // Act 4: a Transit proposed while the old leader is down still completes.
    println!("\n── Act 4: Transit during node failure ──");
    let owner = new_leader;
    let dest = (0..NODES).find(|&i| i != leader && i != owner).unwrap();
    let ship2 = cluster.nodes()[owner]
        .spawn_ship(Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0))
        .await;
    println!("  spawned {ship2:?} in Sector {owner}");
    let accepted = cluster.nodes()[owner].transit(ship2, SectorId(dest as u8)).await;
    println!("  TransitCommand(S{owner} → S{dest}) accepted: {accepted}");
    tick_n(&cluster, 40).await;
    println!("  after 40 ticks:");
    print_stats(&cluster).await;

    // Act 5: heal the partition; the old leader rejoins as Follower.
    println!("\n── Act 5: partition heal ──");
    cluster.heal_node(NodeId(leader as u8));
    tick_n(&cluster, 30).await;
    let roles = cluster.raft_roles().await;
    print_roles("after heal + 30 ticks", &roles);

    cluster.shutdown().await;
    println!("\n  demo complete.");
}

// ── Phase 1: single-node benchmark ───────────────────────────────────────────

fn run_phase1_benchmark() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 1 — single-node benchmark          ");
    println!("═══════════════════════════════════════════");
    println!("  ships : {P1_SHIPS}");
    println!("  ticks : {P1_TICKS}");
    println!();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut node = SimulationNode::new(NodeId(0), SectorId(0), bounds);

    let config = SpawnConfig::default_for_node(NodeId(0));
    let ships  = generate_ships(P1_SHIPS, &config, 0);

    let t0 = Instant::now();
    for (_, pos, vel) in ships {
        node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
    }
    let spawn_ms = t0.elapsed().as_secs_f64() * 1_000.0;

    println!("  spawn  : {P1_SHIPS} ships in {spawn_ms:.2}ms");
    println!();

    let mut tick_times_us: Vec<u128> = Vec::with_capacity(P1_TICKS);
    let mut total_events = 0usize;

    for _ in 0..P1_TICKS {
        let t = Instant::now();
        let r = node.tick();
        tick_times_us.push(t.elapsed().as_micros());
        total_events += r.events_emitted;
    }

    let min_us  = tick_times_us.iter().copied().min().unwrap_or(0);
    let max_us  = tick_times_us.iter().copied().max().unwrap_or(0);
    let mean_us = tick_times_us.iter().sum::<u128>() / tick_times_us.len() as u128;
    let mut sorted = tick_times_us.clone();
    sorted.sort_unstable();
    let p95_us = sorted[(sorted.len() as f64 * 0.95) as usize];

    println!("  ── tick performance ──");
    println!("  min  : {min_us} µs");
    println!("  mean : {mean_us} µs");
    println!("  p95  : {p95_us} µs");
    println!("  max  : {max_us} µs");
    println!("  SLA  : ≤ 16,000 µs  {}",
        if max_us <= 16_000 { "✓ PASS" } else { "✗ FAIL" });
    println!();
    println!("  ── event log ──");
    println!("  total : {}", node.total_event_count());
    println!("  rate  : {:.0} events/sec",
        total_events as f64 / (tick_times_us.iter().sum::<u128>() as f64 / 1_000_000.0));
    println!("═══════════════════════════════════════════");
}

// ── Phase 2: multi-node demo ──────────────────────────────────────────────────

async fn run_phase2_demo() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 2 — in-memory multi-node demo      ");
    println!("═══════════════════════════════════════════");
    println!("  nodes      : {P2_NODES}");
    println!("  ships/node : {P2_SHIPS}");
    println!("  ticks      : {P2_TICKS}");
    println!();

    let cluster = MultiNodeCluster::new(P2_NODES);
    let config  = SpawnConfig::default_for_node(NodeId(0));

    // Spawn ships on all nodes.
    let t0 = Instant::now();
    cluster.spawn_ships_on_all(P2_SHIPS, &config).await;
    println!("  spawn  : {} ships total in {:.2}ms",
        P2_NODES * P2_SHIPS, t0.elapsed().as_secs_f64() * 1_000.0);

    // Run tick loop.
    let t0 = Instant::now();
    for _ in 0..P2_TICKS {
        cluster.tick_all().await;
    }
    let tick_ms = t0.elapsed().as_secs_f64() * 1_000.0;
    println!("  ticks  : {P2_TICKS} ticks × {P2_NODES} nodes in {tick_ms:.2}ms");
    println!();

    // Per-node stats.
    println!("  ── per-node stats ──");
    for stats in cluster.get_all_stats().await {
        println!("  Node({}) Sector({})  ships={}  events={}  tick={}",
            stats.node_id.0, stats.sector_id.0,
            stats.ship_count, stats.event_count, stats.current_tick.value());
    }
    println!();

    // Replication consistency check.
    //
    // ADR-0008: ships only emit a VelocityChanged event when their velocity
    // changes. These NPC ships spawn at a constant velocity and never
    // change it, so the only events are the P2_SHIPS ShipSpawned events
    // per node — not one event per ship per tick.
    let replicated = cluster.total_replicated_events().await;
    let expected   = P2_NODES * P2_SHIPS;

    println!("  ── replication bus ──");
    println!("  replicated : {replicated}");
    println!("  expected   : {expected}  (ShipSpawned only; NPC velocity never changes — ADR-0008)");
    println!("  consistency: {}",
        if replicated == expected { "✓ PASS — all events from all nodes reached the bus" }
        else                      { "✗ FAIL — event loss detected" });

    cluster.shutdown().await;
    println!("═══════════════════════════════════════════");
}

// ── Phase 3: Event persistence demo ───────────────────────────────────────────

const P3_SHIPS : usize = 100;
const P3_TICKS : usize = 10;

fn run_phase3_demo() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 3 — event persistence demo         ");
    println!("═══════════════════════════════════════════");

    let dir           = tempfile::tempdir().expect("failed to create temp dir");
    let event_path    = dir.path().join("sector0_events.log");
    let snapshot_path = dir.path().join("sector0_snapshot.bin");
    let cold_path     = dir.path().join("sector0_cold.log");

    println!("  log      : {}", event_path.display());
    println!("  snapshot : {}", snapshot_path.display());
    println!("  cold     : {}", cold_path.display());
    println!("  ships    : {P3_SHIPS}");
    println!("  ticks    : {P3_TICKS}");
    println!();

    // ── Session 1 ────────────────────────────────────────────────────────────
    let ship_ids: Vec<dawn_core::ShipId>;
    let session1_tick: dawn_core::Tick;
    let session1_positions: Vec<Position>;
    {
        let store = FileEventStore::open(&event_path)
            .expect("failed to open event log");
        let mut node = SimulationNode::with_store(
            NodeId(0), SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            store,
        );

        let config = SpawnConfig::default_for_node(NodeId(0));
        let ships  = generate_ships(P3_SHIPS, &config, 0);
        ship_ids   = ships.iter().map(|(id, ..)| *id).collect();

        for (_, pos, vel) in ships {
            node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        }

        // ADR-0017 8A-7: drive snapshotting + hot-log compaction on a fixed
        // logical-tick cadence. The scheduler saves the authoritative snapshot
        // and compacts the hot log behind it (prefix → cold archive).
        let mut scheduler = checkpoint::CheckpointScheduler::new(checkpoint::CheckpointConfig {
            interval_ticks: (P3_TICKS / 2) as u64,
            snapshot_path : snapshot_path.clone(),
            cold_path     : cold_path.clone(),
        });

        for _ in 0..P3_TICKS {
            node.tick();
            if let Some(snap) = scheduler.maybe_checkpoint(&mut node).expect("checkpoint failed") {
                println!(
                    "  [session 1] checkpoint at tick {}  (log_index={}, hot_base={})",
                    snap.tick.value(), snap.log_index, node.event_store().base_index(),
                );
            }
        }

        session1_tick      = node.current_tick();
        session1_positions = ship_ids.iter()
            .filter_map(|id| node.get_ship_position(*id))
            .collect();
        println!("  [session 1] final tick: {}  events: {}",
            session1_tick.value(), node.total_event_count());
    } // FileEventStore flushes here

    // ── Session 2 (simulated restart) ────────────────────────────────────────
    let snap   = StateSnapshot::load(&snapshot_path).expect("failed to load snapshot");
    let store2 = FileEventStore::open(&event_path).expect("failed to reopen event log");
    let mut node2 = SimulationNode::restore_from(store2, &snap, &[], &[]);

    // ADR-0008: these NPC ships move at a constant velocity and never emit
    // VelocityChanged, so there is nothing to replay past the snapshot's
    // log_index. `restore_from` leaves the node at the snapshot tick; run
    // the remaining ticks (same as session 1 did) to reach session1_tick.
    let remaining = session1_tick.value().saturating_sub(node2.current_tick().value());
    for _ in 0..remaining { node2.tick(); }

    let restored_positions: Vec<Position> = ship_ids.iter()
        .filter_map(|id| node2.get_ship_position(*id))
        .collect();

    let tick_ok = node2.current_tick() == session1_tick;
    let count_ok = node2.ship_count() == P3_SHIPS;
    let pos_ok   = restored_positions == session1_positions;

    println!("  [session 2] restored tick: {}  ships: {}",
        node2.current_tick().value(), node2.ship_count());
    println!();
    println!("  ── consistency check ──");
    println!("  tick match     : {}", if tick_ok   { "✓ PASS" } else { "✗ FAIL" });
    println!("  ship count     : {}", if count_ok  { "✓ PASS" } else { "✗ FAIL" });
    println!("  positions match: {}", if pos_ok    { "✓ PASS" } else { "✗ FAIL" });
    println!("═══════════════════════════════════════════");
}

// ── Phase 5: Godot WebSocket server (multi-client) ─────────────────────────────
//
// Usage:
//   cargo run -p dawn-simulation --bin simulate --release -- --serve
//   cargo run -p dawn-simulation --bin simulate --release -- --serve --ships 10
//
// Changes (ADR-0007):
//   - Assign a PlayerId during the Hello/Welcome handshake
//   - Send all Ship state on connect via InitialState
//   - Support multiple simultaneous client connections
//   - Ownership check: a player can only control their own ship

const P4_SHIPS_DEFAULT : usize = 20;

// ── DuelMetrics ───────────────────────────────────────────────────────────────

/// Per-ship statistics collected during a duel session.
#[derive(Debug, Default)]
struct ShipDuelStats {
    cap_depletions: u32,
}

/// Session-level metrics for duel mode.
/// Accumulated each tick; printed when ShipDestroyed fires.
#[derive(Debug)]
struct DuelMetrics {
    start_tick       : u64,
    /// ship_id → per-ship stats
    stats            : HashMap<ShipId, ShipDuelStats>,
    /// ShipId of the ship that was destroyed (if duel ended)
    loser            : Option<ShipId>,
    /// Tick on which the duel ended
    end_tick         : Option<u64>,
}

impl DuelMetrics {
    fn new(start_tick: u64) -> Self {
        Self {
            start_tick,
            stats    : HashMap::new(),
            loser    : None,
            end_tick : None,
        }
    }

    /// Record cap-forced deactivations for each ship this tick.
    fn record_cap_depletions(&mut self, ship_ids: &[ShipId]) {
        for &id in ship_ids {
            self.stats.entry(id).or_default().cap_depletions += 1;
        }
    }

    /// Record duel end.  `loser` is the ship that was destroyed.
    fn record_end(&mut self, loser: ShipId, tick: u64) {
        self.loser    = Some(loser);
        self.end_tick = Some(tick);
    }

    /// Write a JSON summary to `data/session_<wallclock>.json` for cross-session
    /// balance analysis (playtest-guide.md §6 — `MetricsCollector` minimal version).
    ///
    /// Note: the wall-clock timestamp is used only for the filename / for
    /// human-readable session bookkeeping. Causal ordering inside the
    /// simulation continues to rely solely on the logical Tick (INV-005).
    fn write_json_summary(&self, player_ship_id: Option<ShipId>) {
        let duration = self.end_tick.unwrap_or(self.start_tick) - self.start_tick;

        let ships: Vec<serde_json::Value> = {
            let mut ids: Vec<ShipId> = self.stats.keys().cloned().collect();
            ids.sort_by_key(|id| id.raw());
            ids.iter().map(|id| {
                let s = &self.stats[id];
                serde_json::json!({
                    "ship_id": id.raw(),
                    "role": if player_ship_id == Some(*id) { "player" } else { "bot" },
                    "cap_depletions": s.cap_depletions,
                })
            }).collect()
        };

        let result = self.loser.map(|loser| {
            let player_won = player_ship_id.map_or(false, |pid| pid != loser);
            if player_won { "player_win" } else { "bot_win" }
        });

        let summary = serde_json::json!({
            "mode": "duel",
            "start_tick": self.start_tick,
            "end_tick": self.end_tick,
            "duration_ticks": duration,
            "result": result,
            "loser_ship_id": self.loser.map(|id| id.raw()),
            "ships": ships,
        });

        let wall_clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = std::path::Path::new("data");
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("  [Duel] failed to create data/ directory: {e}");
            return;
        }
        let path = dir.join(format!("session_duel_{wall_clock}.json"));
        match serde_json::to_string_pretty(&summary) {
            Ok(text) => match std::fs::write(&path, text) {
                Ok(()) => println!("  [Duel] session summary written to {}", path.display()),
                Err(e) => eprintln!("  [Duel] failed to write {}: {e}", path.display()),
            },
            Err(e) => eprintln!("  [Duel] failed to serialize summary: {e}"),
        }
    }

    /// Print a formatted summary to stdout.
    fn print_summary(&self, player_ship_id: Option<ShipId>) {
        let duration = self.end_tick.unwrap_or(self.start_tick) - self.start_tick;

        println!();
        println!("╔══════════════════════════════════════════╗");
        println!("║           DUEL RESULT                    ║");
        println!("╠══════════════════════════════════════════╣");

        if let Some(loser) = self.loser {
            let player_won = player_ship_id.map_or(false, |pid| pid != loser);
            let result_str = if player_won { "PLAYER WIN" } else { "BOT WIN" };
            println!("║  Result  : {:<31}║", result_str);
        }

        println!("║  Duration: {:<3} ticks                      ║", duration);
        println!("╠══════════════════════════════════════════╣");
        println!("║  Ship  │  Cap Depletions                  ║");
        println!("║  ──────┼──────────────────────────────── ║");

        let mut ids: Vec<ShipId> = self.stats.keys().cloned().collect();
        ids.sort_by_key(|id| id.raw());
        for id in &ids {
            let s = &self.stats[id];
            let label = if player_ship_id == Some(*id) { "Player" } else { "Bot   " };
            println!("║  #{:<4} ({}) │  cap deplete ×{:<18}║",
                id.raw(), label, s.cap_depletions);
        }
        if ids.is_empty() {
            println!("║  (no data)                               ║");
        }

        println!("╚══════════════════════════════════════════╝");
        println!();
    }
}
const P4_TICK_MS       : u64   = 100;  // 10 Tick/sec

/// Apply a player command that behaves identically in the single-node
/// (`--serve`) and clustered (`--serve --cluster`) servers — every command
/// except `Jump`, whose handling differs (ignored vs. proposed to Raft).
///
/// Accepted `LockOn` commands are pushed to `lock_commands` for the tick's
/// Lock System. When `cmd` is a `Jump`, it is returned to the caller so each
/// server can route it appropriately instead of being handled here.
fn apply_common_command(
    node         : &mut SimulationNode,
    player_id    : dawn_core::PlayerId,
    cmd          : ClientCommand,
    lock_commands: &mut Vec<dawn_core::LockOnCommand>,
) -> Option<dawn_core::JumpCommand> {
    match cmd {
        ClientCommand::Move(mv) => {
            node.apply_move_command_owned(player_id, mv.ship_id, mv.target_position);
        }
        ClientCommand::LockOn(lo) => {
            if node.apply_lock_on_owned(player_id, lo.clone()) {
                lock_commands.push(lo);
            }
        }
        ClientCommand::Activate(c)   => { node.activate_module_owned(player_id, c); }
        ClientCommand::Deactivate(c) => { node.deactivate_module_owned(player_id, c); }
        // Combat is automatic (CombatSystem each tick); AttackCommand is
        // reserved for a future manual-fire mode.
        ClientCommand::Attack(_) => {}
        ClientCommand::Stop(s) => { node.apply_stop_command_owned(player_id, s.ship_id); }
        // Approach: semi-automatic piloting toward a chosen ship/gate (ADR-0015).
        ClientCommand::Approach(a) => { node.apply_approach_command_owned(player_id, a); }
        // Jump differs per server: hand it back to the caller.
        ClientCommand::Jump(j) => return Some(j),
    }
    None
}

async fn run_phase4_server(ship_count: usize, duel_mode: bool) {
    println!("═══════════════════════════════════════════");
    println!("  Phase 5 — Godot WebSocket server          ");
    println!("═══════════════════════════════════════════");
    if duel_mode {
        println!("  mode: DUEL (1 human vs 1 Bot, no NPC)");
    } else {
        println!("  npc ships: {ship_count}  (change with --ships N)");
    }
    println!("  tick rate: {} ms/tick  ({} tick/sec)",
        P4_TICK_MS, 1000 / P4_TICK_MS);
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = WsServer::bind("127.0.0.1:7878").await
        .expect("failed to bind WebSocket server");

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut node = SimulationNode::new(NodeId(0), SectorId(0), bounds);

    let loaded_modules = data_loader::load_modules(
        "data/modules.toml",
        modules::all_modules(),
    );
    for def in loaded_modules {
        node.register_module(def);
    }

    let loaded_ship_types = data_loader::load_ship_types(
        "data/ship_types.toml",
        ship_types::all_ship_types(),
    );
    for def in loaded_ship_types {
        node.register_ship_type(def);
    }

    // Generate NPC ships
    let config = SpawnConfig::default_for_node(NodeId(0));
    let ships  = generate_ships(ship_count, &config, 0);
    for (_, pos, vel) in ships {
        let ship_id = node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        node.fit_module(dawn_core::FitModuleCommand {
            ship_id,
            slot      : dawn_core::SlotKind::High,
            module_id : modules::MODULE_RAILGUN_SMALL,
        });
    }
    // Duel mode: spawn 1 Bot opposite the player's default spawn position.
    if duel_mode {
        let bot_pos = Position::new(1200.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);
        println!("  [Server] Duel mode: Bot ship #{} ready at {:?}", bot_ship_id.raw(), bot_pos);
    }

    println!("  [Server] {ship_count} NPC ships ready. Waiting for players...");

    // TCP connection channel (accept task -> main loop)
    let (new_conn_tx, mut new_conn_rx) =
        mpsc::unbounded_channel::<(tokio::net::TcpStream, std::net::SocketAddr)>();

    // Handshake-completed channel (handshake task -> main loop)
    // Created outside the loop so it isn't dropped and still receives after handshakes complete
    let (ready_sess_tx, mut ready_sess_rx) =
        mpsc::unbounded_channel::<ws_server::PlayerSession>();

    // Run the accept loop in a separate task
    let server_arc = std::sync::Arc::new(server);
    let server_clone = server_arc.clone();
    tokio::spawn(async move {
        loop {
            if let Some((stream, addr)) = server_clone.try_accept_raw().await {
                let _ = new_conn_tx.send((stream, addr));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    let mut sessions: Vec<ws_server::PlayerSession> = Vec::new();
    let mut interval = tokio::time::interval(
        std::time::Duration::from_millis(P4_TICK_MS)
    );

    // Duel mode: track session metrics (None until the player connects).
    let mut duel_metrics  : Option<DuelMetrics> = None;
    let mut player_ship_id: Option<ShipId>      = None;

    loop {
        interval.tick().await;

        // New TCP connection → spawn handshake task.
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            let player_id      = node.next_player_id();
            let ship_id        = node.spawn_player_ship(player_id);
            let initial_state  = node.build_initial_state_json();
            let player_fitting = node.build_player_fitting_json(ship_id);
            let tx             = ready_sess_tx.clone();

            // Duel mode: start metric collection when the first player connects.
            if duel_mode && player_ship_id.is_none() {
                player_ship_id = Some(ship_id);
                let tick = node.current_tick().value();
                duel_metrics = Some(DuelMetrics::new(tick));
                println!("  [Duel] metrics collection started at tick {tick}");
            }

            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream, addr, player_id, ship_id, &initial_state, player_fitting
                ).await {
                    Ok(sess) => { let _ = tx.send(sess); }
                    Err(e)   => eprintln!("[Server] handshake failed: {e}"),
                }
            });
        }

        // Add handshake-completed sessions.
        while let Ok(sess) = ready_sess_rx.try_recv() {
            println!("  [Server] {} joined with ship #{}", sess.player_id, sess.ship_id.raw());
            sessions.push(sess);
        }

        // Record event-store position before command processing so that
        // command-driven events (ModuleActivated etc.) are also broadcast.
        let events_before: u64 = node.total_event_count() as u64;

        // Collect commands (with ownership check).
        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();

        for sess in sessions.iter_mut() {
            while let Some(cmd) = sess.try_recv_command() {
                if let Some(j) = apply_common_command(&mut node, sess.player_id, cmd, &mut lock_commands) {
                    // Jump Gate Transit requires the Raft pipeline (FBD-006);
                    // the --serve demo runs a single Sector-0 node without a
                    // cluster, so Jump commands are ignored here. The full
                    // pipeline is exercised by --serve --cluster (ADR-0009).
                    eprintln!(
                        "[Server] JumpCommand ignored (ship #{} gate #{}): \
                         --serve runs a single-sector node without Raft",
                        j.ship_id.raw(), j.gate_id.0
                    );
                }
            }
        }

        // Run one tick.
        let tick_result = node.tick_with_lock_commands(&lock_commands);

        // Duel metrics: accumulate cap depletions and detect duel end.
        if duel_mode {
            if let Some(ref mut metrics) = duel_metrics {
                metrics.record_cap_depletions(&tick_result.cap_depletions);

                // Check for ShipDestroyed in this tick's events.
                for event in &tick_result.events {
                    if let dawn_core::DomainEvent::ShipDestroyed(e) = event {
                        metrics.record_end(e.ship_id, tick_result.tick.value());
                        metrics.print_summary(player_ship_id);
                        metrics.write_json_summary(player_ship_id);
                        // Continue running so the client can display the result.
                    }
                }
            }
        }

        // Broadcast all new events (command-driven + tick) to all clients.
        let all_new_events: Vec<_> = {
            use dawn_event_store::store::EventStore as _;
            node.event_store().iter_from(events_before)
                .map(|r| r.event.clone())
                .collect()
        };
        sessions.retain(|sess| sess.send_events(&all_new_events));
    }
}

// ── Phase 7.5: --serve --cluster (Jump Gates over Raft, ADR-0009) ─────────────

/// Godot WebSocket server backed by a 3-node Raft cluster, one node per
/// Sector (0/1/2), so `JumpCommand` runs the full ADR-0009 pipeline:
/// validation → `TransitOp::Request` on the Raft Log → Step 7.5 apply →
/// `JumpGateUsed` / `StarSystemChanged` broadcast to the client.
///
/// Differences from `run_phase4_server`:
/// - 3 `SimulationNode`s ticked in lockstep, each with its own `RaftActor`
///   (in-process transport, same wiring as `MultiNodeCluster`).
/// - Player commands are routed to the node that currently owns the
///   player's ship; ownership follows `JumpGateUsed` events.
/// - Players spawn near Gate 0 (Sector 0 → Sector 1) so the jump can be
///   tried immediately.
async fn run_cluster_server(ship_count: usize) {
    use dawn_core::{DomainEvent, PlayerId};
    use dawn_event_store::store::EventStore as _;
    use transit::TransitOp;

    const SECTORS: usize = 3;
    /// In range of Gate 0 (position x=49,000, activation_radius 2,000).
    const PLAYER_SPAWN: Position = Position { x: 47_500.0, y: 0.0, z: 0.0 };

    println!("═══════════════════════════════════════════");
    println!("  Phase 7.5 — Raft cluster WebSocket server ");
    println!("═══════════════════════════════════════════");
    println!("  sectors  : {SECTORS} (one Raft node each)");
    println!("  npc ships: {ship_count} in Sector 0  (change with --ships N)");
    println!("  tick rate: {} ms/tick  ({} tick/sec)", P4_TICK_MS, 1000 / P4_TICK_MS);
    println!("  jump     : press J near a gate (player spawns near Gate 0)");
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = WsServer::bind("127.0.0.1:7878").await
        .expect("failed to bind WebSocket server");

    // One SimulationNode + RaftActor per Sector (ADR-0014 wiring).
    let ids: Vec<NodeId> = (0..SECTORS as u8).map(NodeId).collect();
    let (endpoints, _partitioned) = cluster::spawn_raft_actors(&ids);
    let (rafts, mut committed_rxs): (Vec<_>, Vec<_>) = endpoints.into_iter().unzip();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut nodes: Vec<SimulationNode> = ids.iter().map(|&id| {
        let mut node = SimulationNode::new(id, SectorId(id.0), bounds);
        for def in data_loader::load_modules("data/modules.toml", modules::all_modules()) {
            node.register_module(def);
        }
        for def in data_loader::load_ship_types("data/ship_types.toml", ship_types::all_ship_types()) {
            node.register_ship_type(def);
        }
        node
    }).collect();

    // NPC ships live in Sector 0 only.
    let config = SpawnConfig::default_for_node(NodeId(0));
    for (_, pos, vel) in generate_ships(ship_count, &config, 0) {
        let ship_id = nodes[0].spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        nodes[0].fit_module(dawn_core::FitModuleCommand {
            ship_id,
            slot      : dawn_core::SlotKind::High,
            module_id : modules::MODULE_RAILGUN_SMALL,
        });
    }

    // Warm up: tick until a Raft leader is elected so early Jump proposals
    // are not dropped (election timeout is at most 20 ticks).
    for _ in 0..30 {
        for i in 0..SECTORS {
            transit::apply_committed_raft_entries(&mut nodes[i], &rafts[i], &mut committed_rxs[i]);
            nodes[i].tick();
            rafts[i].tick();
        }
    }
    println!("  [Server] Raft warm-up complete. Waiting for players...");

    let (new_conn_tx, mut new_conn_rx) =
        mpsc::unbounded_channel::<(tokio::net::TcpStream, std::net::SocketAddr)>();
    let (ready_sess_tx, mut ready_sess_rx) =
        mpsc::unbounded_channel::<ws_server::PlayerSession>();

    let server_arc = std::sync::Arc::new(server);
    let server_clone = server_arc.clone();
    tokio::spawn(async move {
        loop {
            if let Some((stream, addr)) = server_clone.try_accept_raw().await {
                let _ = new_conn_tx.send((stream, addr));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    let mut sessions: Vec<ws_server::PlayerSession> = Vec::new();
    // Which Sector currently owns each player's ship (updated on JumpGateUsed).
    let mut player_sector: HashMap<PlayerId, usize> = HashMap::new();
    let mut ship_player  : HashMap<ShipId, PlayerId> = HashMap::new();

    let mut interval = tokio::time::interval(
        std::time::Duration::from_millis(P4_TICK_MS)
    );

    loop {
        interval.tick().await;

        // New TCP connection → spawn player in Sector 0 near Gate 0.
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            let player_id      = nodes[0].next_player_id();
            let ship_id        = nodes[0].spawn_player_ship_at_pub(player_id, PLAYER_SPAWN);
            let initial_state  = nodes[0].build_initial_state_json();
            let player_fitting = nodes[0].build_player_fitting_json(ship_id);
            let tx             = ready_sess_tx.clone();
            player_sector.insert(player_id, 0);
            ship_player.insert(ship_id, player_id);

            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream, addr, player_id, ship_id, &initial_state, player_fitting
                ).await {
                    Ok(sess) => { let _ = tx.send(sess); }
                    Err(e)   => eprintln!("[Server] handshake failed: {e}"),
                }
            });
        }

        while let Ok(sess) = ready_sess_rx.try_recv() {
            println!("  [Server] {} joined with ship #{}", sess.player_id, sess.ship_id.raw());
            sessions.push(sess);
        }

        // Record per-node event-store positions before command processing so
        // command-driven events (ModuleActivated etc.) are also broadcast.
        let events_before: Vec<u64> =
            nodes.iter().map(|n| n.total_event_count() as u64).collect();

        // Collect commands, routed to the node that owns the player's ship.
        let mut lock_commands: Vec<Vec<dawn_core::LockOnCommand>> =
            vec![Vec::new(); SECTORS];

        for sess in sessions.iter_mut() {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            while let Some(cmd) = sess.try_recv_command() {
                let Some(j) = apply_common_command(&mut nodes[sector], sess.player_id, cmd, &mut lock_commands[sector])
                else { continue };
                // ADR-0009: validate up front, then propose to the Raft Log;
                // the move happens once the entry commits (Step 7.5).
                let accepted = j.ship_id == sess.ship_id
                    && nodes[sector].can_propose_jump(j.ship_id, j.gate_id);
                if accepted {
                    let to = nodes[sector].jump_gate(j.gate_id)
                        .expect("can_propose_jump confirmed gate exists")
                        .to_sector;
                    rafts[sector].propose(
                        TransitOp::Request { ship_id: j.ship_id, to, gate_id: Some(j.gate_id) }.encode(),
                    );
                    println!("  [Server] Jump proposed: ship #{} gate #{} (S{} → S{})",
                        j.ship_id.raw(), j.gate_id.0, sector, to.0);
                } else {
                    eprintln!("[Server] JumpCommand rejected (ship #{} gate #{})",
                        j.ship_id.raw(), j.gate_id.0);
                }
            }
        }

        // Tick every node: Step 7.5 (apply committed Raft entries) →
        // simulation step → Step 10 (advance Raft timers).
        for i in 0..SECTORS {
            transit::apply_committed_raft_entries(&mut nodes[i], &rafts[i], &mut committed_rxs[i]);
            nodes[i].tick_with_lock_commands(&lock_commands[i]);
            rafts[i].tick();
        }

        // Collect new events per Sector so each client only sees the Sector
        // its ship is currently in — Sectors are independent worlds and a
        // player in Sector 1 must not receive Sector 0's NPC events.
        let events_by_sector: Vec<Vec<DomainEvent>> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                node.event_store().iter_from(events_before[i]).map(|r| r.event.clone()).collect()
            })
            .collect();

        // Ownership handoff: a player ship completed a jump → the destination
        // node adopts it, future commands route there, and we remember to
        // resend that Sector's InitialState below.
        let mut jumped_players: Vec<(PlayerId, usize)> = Vec::new();
        for sector_events in &events_by_sector {
            for event in sector_events {
                if let DomainEvent::JumpGateUsed(e) = event {
                    if let Some(&player_id) = ship_player.get(&e.ship_id) {
                        let dest = e.to_sector.0 as usize;
                        nodes[dest].adopt_player_ship(e.ship_id, player_id);
                        player_sector.insert(player_id, dest);
                        jumped_players.push((player_id, dest));
                        println!("  [Server] {player_id:?} ship #{} now owned by Sector {dest}",
                            e.ship_id.raw());
                    }
                }
            }
        }

        // Send each session only the events from the Sector it now occupies.
        sessions.retain(|sess| {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            sess.send_events(&events_by_sector[sector])
        });

        // Resend InitialState to players that just jumped so the client clears
        // the old Sector's ships (and its stale lock) and rebuilds from the
        // destination Sector. `_on_initial_state` resets `_player_lock_target`.
        for (player_id, dest) in jumped_players {
            let initial_state = nodes[dest].build_initial_state_json();
            if let Some(sess) = sessions.iter().find(|s| s.player_id == player_id) {
                sess.conn.send_raw(&initial_state);
            }
        }
    }
}
