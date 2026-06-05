//! Simulation entry point.
//!
//! Phase 1: single-node benchmark  (10,000 ships × 100 ticks)
//! Phase 2: multi-node demo        (3 nodes × 1,000 ships × 20 ticks)
//!
//! Usage:
//!   cargo run -p dawn-simulation --bin simulate
//!   cargo run -p dawn-simulation --bin simulate --release

mod cluster;
mod modules;
mod node;
mod ship_types;
mod sector_simulator_actor;
mod snapshot;
mod spawner;
mod ws_server;

use dawn_actor::ClientCommand;
use dawn_core::{NodeId, SectorBounds, SectorId};
use tokio::sync::mpsc;
use node::SimulationNode;
use spawner::{generate_ships, SpawnConfig};
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
    // Phase 4 モード: --serve 引数があれば Godot 向け WebSocket サーバーを起動
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--serve".to_string()) {
        // --ships N で船の数を指定できる（省略時は P4_SHIPS_DEFAULT）
        let ship_count = args.windows(2)
            .find(|w| w[0] == "--ships")
            .and_then(|w| w[1].parse::<usize>().ok())
            .unwrap_or(P4_SHIPS_DEFAULT);
        run_phase4_server(ship_count).await;
        return;
    }

    run_phase1_benchmark();
    println!();
    run_phase2_demo().await;
    println!();
    run_phase3_demo();
}

// ── Phase 1: single-node benchmark ───────────────────────────────────────────

fn run_phase1_benchmark() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 1 — single-node benchmark          ");
    println!("═══════════════════════════════════════════");
    println!("  ships : {P1_SHIPS}");
    println!("  ticks : {P1_TICKS}");
    println!();

    let bounds = SectorBounds::cube(SectorBounds::DEFAULT_SIZE);
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
    let replicated = cluster.total_replicated_events().await;
    let expected   = P2_NODES * (P2_SHIPS + P2_SHIPS * P2_TICKS);

    println!("  ── replication bus ──");
    println!("  replicated : {replicated}");
    println!("  expected   : {expected}");
    println!("  consistency: {}",
        if replicated == expected { "✓ PASS — all events from all nodes reached the bus" }
        else                      { "✗ FAIL — event loss detected" });

    cluster.shutdown().await;
    println!("═══════════════════════════════════════════");
}

// ── Phase 3: Event 永続化デモ ─────────────────────────────────────────────────

const P3_SHIPS : usize = 100;
const P3_TICKS : usize = 10;

fn run_phase3_demo() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 3 — event persistence demo         ");
    println!("═══════════════════════════════════════════");

    let dir           = tempfile::tempdir().expect("failed to create temp dir");
    let event_path    = dir.path().join("sector0_events.log");
    let snapshot_path = dir.path().join("sector0_snapshot.bin");

    println!("  log      : {}", event_path.display());
    println!("  snapshot : {}", snapshot_path.display());
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
            SectorBounds::cube(SectorBounds::DEFAULT_SIZE),
            store,
        );

        let config = SpawnConfig::default_for_node(NodeId(0));
        let ships  = generate_ships(P3_SHIPS, &config, 0);
        ship_ids   = ships.iter().map(|(id, ..)| *id).collect();

        for (_, pos, vel) in ships {
            node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        }

        // Run half the ticks, take snapshot.
        for _ in 0..(P3_TICKS / 2) { node.tick(); }
        let snap = node.take_snapshot();
        snap.save(&snapshot_path).expect("failed to save snapshot");
        println!("  [session 1] snapshot taken at tick {}  (log_index={})",
            snap.tick.value(), snap.log_index);

        // Run remaining ticks.
        for _ in 0..(P3_TICKS - P3_TICKS / 2) { node.tick(); }

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
    let node2  = SimulationNode::restore_from(store2, &snap);

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

// ── Phase 5: Godot WebSocket サーバー（マルチクライアント対応）──────────────
//
// 使い方:
//   cargo run -p dawn-simulation --bin simulate --release -- --serve
//   cargo run -p dawn-simulation --bin simulate --release -- --serve --ships 10
//
// 変更点 (ADR-0007):
//   - Hello/Welcome ハンドシェイクで PlayerId を採番
//   - InitialState で接続時の全 Ship 状態を送信
//   - 複数クライアントの同時接続に対応
//   - 所有権チェック: 自分の船だけ操作できる

const P4_SHIPS_DEFAULT : usize = 20;
const P4_TICK_MS       : u64   = 100;  // 10 Tick/sec

async fn run_phase4_server(ship_count: usize) {
    println!("═══════════════════════════════════════════");
    println!("  Phase 5 — Godot WebSocket server          ");
    println!("═══════════════════════════════════════════");
    println!("  npc ships: {ship_count}  (change with --ships N)");
    println!("  tick rate: {} ms/tick  ({} tick/sec)",
        P4_TICK_MS, 1000 / P4_TICK_MS);
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = WsServer::bind("127.0.0.1:7878").await
        .expect("failed to bind WebSocket server");

    let bounds = SectorBounds::cube(SectorBounds::DEFAULT_SIZE);
    let mut node = SimulationNode::new(NodeId(0), SectorId(0), bounds);

    for def in modules::all_modules() {
        node.register_module(def);
    }
    for def in ship_types::all_ship_types() {
        node.register_ship_type(def);
    }

    // NPC 船を生成
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
    println!("  [Server] {ship_count} NPC ships ready. Waiting for players...");

    // TCP 接続チャンネル（accept タスク → メインループ）
    let (new_conn_tx, mut new_conn_rx) =
        mpsc::unbounded_channel::<(tokio::net::TcpStream, std::net::SocketAddr)>();

    // ハンドシェイク完了チャンネル（handshake タスク → メインループ）
    // ループ外で生成することでドロップされず、ハンドシェイク完了後も受け取れる
    let (ready_sess_tx, mut ready_sess_rx) =
        mpsc::unbounded_channel::<ws_server::PlayerSession>();

    // accept ループを別タスクで実行
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

    loop {
        interval.tick().await;

        // 新規 TCP 接続 → ハンドシェイクタスクを spawn
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            let player_id      = node.next_player_id();
            let ship_id        = node.spawn_player_ship(player_id);
            let initial_state  = node.build_initial_state_json();
            let player_fitting = node.build_player_fitting_json(ship_id);
            let tx             = ready_sess_tx.clone();
            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream, addr, player_id, ship_id, &initial_state, player_fitting
                ).await {
                    Ok(sess) => { let _ = tx.send(sess); }
                    Err(e)   => eprintln!("[Server] handshake failed: {e}"),
                }
            });
        }

        // ハンドシェイク完了したセッションを sessions に追加
        while let Ok(sess) = ready_sess_rx.try_recv() {
            println!("  [Server] {} joined with ship #{}", sess.player_id, sess.ship_id.raw());
            sessions.push(sess);
        }

        // コマンド処理前のイベントストア位置を記録
        // ModuleActivated 等のコマンド由来イベントも含めて送信するため
        let events_before: u64 = node.total_event_count() as u64;

        // コマンド収集（所有権チェック付き）
        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();

        for sess in sessions.iter_mut() {
            while let Some(cmd) = sess.try_recv_command() {
                match cmd {
                    ClientCommand::Move(mv) => {
                        node.apply_move_command_owned(sess.player_id, mv.ship_id, mv.target_position);
                    }
                    ClientCommand::LockOn(lo) => {
                        if node.apply_lock_on_owned(sess.player_id, lo.clone()) {
                            lock_commands.push(lo);
                        }
                    }
                    ClientCommand::Activate(cmd) => {
                        node.activate_module_owned(sess.player_id, cmd);
                    }
                    ClientCommand::Deactivate(cmd) => {
                        node.deactivate_module_owned(sess.player_id, cmd);
                    }
                }
            }
        }

        // Tick 実行
        node.tick_with_lock_commands(&lock_commands);

        // コマンド由来イベント（ModuleActivated 等）＋ Tick イベントを全てブロードキャスト
        let all_new_events: Vec<_> = {
            use dawn_event_store::store::EventStore as _;
            node.event_store().iter_from(events_before)
                .map(|r| r.event.clone())
                .collect()
        };
        sessions.retain(|sess| sess.send_events(&all_new_events));

    }
}
