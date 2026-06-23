//! Benchmark and demo functions (Phases 1–3, AoI, Raft demo).

use crate::cluster::MultiNodeCluster;
use crate::serve::AOI_CELL_SIZE;
use dawn_core::{NodeId, Position, SectorBounds, SectorId, Velocity};
use dawn_event_store::FileEventStore;
use dawn_sector::node::SimulationNode;
use dawn_sector::persistence::StateSnapshot;
use dawn_sector::spawner::{generate_ships, SpawnConfig};
use dawn_sector::{aoi, persistence, ship_types};
use std::time::Instant;

// ── Constants ─────────────────────────────────────────────────────────────────

pub(crate) const P1_SHIPS: usize = 10_000;
pub(crate) const P1_TICKS: usize = 100;

pub(crate) const P2_NODES: usize = 3;
pub(crate) const P2_SHIPS: usize = 1_000; // per node
pub(crate) const P2_TICKS: usize = 20;

const P3_SHIPS: usize = 100;
const P3_TICKS: usize = 10;

// ── Phase 7: Raft Transit demo ───────────────────────────────────────────────

/// Observable walkthrough of the Phase 7 pipeline (ADR-0014):
/// leader election → Transit through the Raft Log → leader failover →
/// Transit completing under a partitioned old leader → partition heal.
pub(crate) async fn run_raft_demo() {
    use dawn_consensus::Role;

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
        cluster
            .raft_roles()
            .await
            .iter()
            .enumerate()
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
    let leader = leader_index(&cluster, None)
        .await
        .expect("a leader must be elected");
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
    let new_leader = leader_index(&cluster, Some(leader))
        .await
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
    let accepted = cluster.nodes()[owner]
        .transit(ship2, SectorId(dest as u8))
        .await;
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

pub(crate) fn run_phase1_benchmark() {
    use dawn_core::SectorBounds;

    println!("═══════════════════════════════════════════");
    println!("  Phase 1 — single-node benchmark          ");
    println!("═══════════════════════════════════════════");
    println!("  ships : {P1_SHIPS}");
    println!("  ticks : {P1_TICKS}");
    println!();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut node = SimulationNode::new(NodeId(0), SectorId(0), bounds);

    let config = SpawnConfig::default_for_node(NodeId(0));
    let ships = generate_ships(P1_SHIPS, &config, 0);

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

    let min_us = tick_times_us.iter().copied().min().unwrap_or(0);
    let max_us = tick_times_us.iter().copied().max().unwrap_or(0);
    let mean_us = tick_times_us.iter().sum::<u128>() / tick_times_us.len() as u128;
    let mut sorted = tick_times_us.clone();
    sorted.sort_unstable();
    let p95_us = sorted[(sorted.len() as f64 * 0.95) as usize];

    println!("  ── tick performance ──");
    println!("  min  : {min_us} µs");
    println!("  mean : {mean_us} µs");
    println!("  p95  : {p95_us} µs");
    println!("  max  : {max_us} µs");
    println!(
        "  SLA  : ≤ 16,000 µs  {}",
        if max_us <= 16_000 {
            "✓ PASS"
        } else {
            "✗ FAIL"
        }
    );
    println!();
    println!("  ── event log ──");
    println!("  total : {}", node.total_event_count());
    println!(
        "  rate  : {:.0} events/sec",
        total_events as f64 / (tick_times_us.iter().sum::<u128>() as f64 / 1_000_000.0)
    );
    println!("═══════════════════════════════════════════");
}

// ── Phase 2: multi-node demo ──────────────────────────────────────────────────

pub(crate) async fn run_phase2_demo() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 2 — in-memory multi-node demo      ");
    println!("═══════════════════════════════════════════");
    println!("  nodes      : {P2_NODES}");
    println!("  ships/node : {P2_SHIPS}");
    println!("  ticks      : {P2_TICKS}");
    println!();

    let cluster = MultiNodeCluster::new(P2_NODES);
    let config = SpawnConfig::default_for_node(NodeId(0));

    // Spawn ships on all nodes.
    let t0 = Instant::now();
    cluster.spawn_ships_on_all(P2_SHIPS, &config).await;
    println!(
        "  spawn  : {} ships total in {:.2}ms",
        P2_NODES * P2_SHIPS,
        t0.elapsed().as_secs_f64() * 1_000.0
    );

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
        println!(
            "  Node({}) Sector({})  ships={}  events={}  tick={}",
            stats.node_id.0,
            stats.sector_id.0,
            stats.ship_count,
            stats.event_count,
            stats.current_tick.value()
        );
    }
    println!();

    // Replication consistency check.
    //
    // ADR-0008: ships only emit a VelocityChanged event when their velocity
    // changes. These NPC ships spawn at a constant velocity and never
    // change it, so the only events are the P2_SHIPS ShipSpawned events
    // per node — not one event per ship per tick.
    let replicated = cluster.total_replicated_events().await;
    let expected = P2_NODES * P2_SHIPS;

    println!("  ── replication bus ──");
    println!("  replicated : {replicated}");
    println!(
        "  expected   : {expected}  (ShipSpawned only; NPC velocity never changes — ADR-0008)"
    );
    println!(
        "  consistency: {}",
        if replicated == expected {
            "✓ PASS — all events from all nodes reached the bus"
        } else {
            "✗ FAIL — event loss detected"
        }
    );

    cluster.shutdown().await;
    println!("═══════════════════════════════════════════");
}

// ── Phase 3: Event persistence demo ───────────────────────────────────────────

pub(crate) fn run_phase3_demo() {
    println!("═══════════════════════════════════════════");
    println!("  Phase 3 — event persistence demo         ");
    println!("═══════════════════════════════════════════");

    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let event_path = dir.path().join("sector0_events.log");
    let snapshot_path = dir.path().join("sector0_snapshot.bin");
    let cold_path = dir.path().join("sector0_cold.log");

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
        let store = FileEventStore::open(&event_path).expect("failed to open event log");
        let mut node = SimulationNode::with_store(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            store,
        );

        let config = SpawnConfig::default_for_node(NodeId(0));
        let ships = generate_ships(P3_SHIPS, &config, 0);
        ship_ids = ships.iter().map(|(id, ..)| *id).collect();

        for (_, pos, vel) in ships {
            node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        }

        // ADR-0017 8A-7: drive snapshotting + hot-log compaction on a fixed
        // logical-tick cadence. The scheduler saves the authoritative snapshot
        // and compacts the hot log behind it (prefix → cold archive).
        let mut scheduler = persistence::CheckpointScheduler::new(persistence::CheckpointConfig {
            interval_ticks: (P3_TICKS / 2) as u64,
            snapshot_path: snapshot_path.clone(),
            cold_path: cold_path.clone(),
        });

        for _ in 0..P3_TICKS {
            node.tick();
            if let Some(snap) = scheduler
                .maybe_checkpoint(&mut node)
                .expect("checkpoint failed")
            {
                println!(
                    "  [session 1] checkpoint at tick {}  (log_index={}, hot_base={})",
                    snap.tick.value(),
                    snap.log_index,
                    node.event_store().base_index(),
                );
            }
        }

        session1_tick = node.current_tick();
        session1_positions = ship_ids
            .iter()
            .filter_map(|id| node.get_ship_position(*id))
            .collect();
        println!(
            "  [session 1] final tick: {}  events: {}",
            session1_tick.value(),
            node.total_event_count()
        );
    } // FileEventStore flushes here

    // ── Session 2 (simulated restart) ────────────────────────────────────────
    let snap = StateSnapshot::load(&snapshot_path).expect("failed to load snapshot");
    let store2 = FileEventStore::open(&event_path).expect("failed to reopen event log");
    let mut node2 = SimulationNode::restore_from(store2, &snap, &[], &[]);

    // ADR-0008: these NPC ships move at a constant velocity and never emit
    // VelocityChanged, so there is nothing to replay past the snapshot's
    // log_index. `restore_from` leaves the node at the snapshot tick; run
    // the remaining ticks (same as session 1 did) to reach session1_tick.
    let remaining = session1_tick
        .value()
        .saturating_sub(node2.current_tick().value());
    for _ in 0..remaining {
        node2.tick();
    }

    let restored_positions: Vec<Position> = ship_ids
        .iter()
        .filter_map(|id| node2.get_ship_position(*id))
        .collect();

    let tick_ok = node2.current_tick() == session1_tick;
    let count_ok = node2.ship_count() == P3_SHIPS;
    let pos_ok = restored_positions == session1_positions;

    println!(
        "  [session 2] restored tick: {}  ships: {}",
        node2.current_tick().value(),
        node2.ship_count()
    );
    println!();
    println!("  ── consistency check ──");
    println!(
        "  tick match     : {}",
        if tick_ok { "✓ PASS" } else { "✗ FAIL" }
    );
    println!(
        "  ship count     : {}",
        if count_ok { "✓ PASS" } else { "✗ FAIL" }
    );
    println!(
        "  positions match: {}",
        if pos_ok { "✓ PASS" } else { "✗ FAIL" }
    );
    println!("═══════════════════════════════════════════");
}

// ── AoI scaling benchmark (ADR-0019, 8C-6) ────────────────────────────────────

/// Demonstrate that the static cell grid raises the single-Sector capacity:
/// the per-tick interest cost (grid build + one neighbor query per observer)
/// and the delivery volume stay bounded by *local* density, not the global
/// ship count — whereas the no-AoI baseline grows as p·n (→ O(n²) when p≈n).
pub(crate) fn run_aoi_benchmark() {
    use dawn_core::ShipId;

    println!("═══════════════════════════════════════════");
    println!("  AoI scaling benchmark (ADR-0019)         ");
    println!("═══════════════════════════════════════════");
    let half = SectorBounds::DEFAULT_HALF;
    let span = 2.0 * half;
    println!(
        "  sector : {span:.0}^3 units   cell: {AOI_CELL_SIZE:.0}   (10% of ships are observers)"
    );
    println!();
    println!(
        "  {:>7} {:>6} | {:>11} {:>11} | {:>10} {:>9} | {:>9}",
        "ships", "obs", "AoI build", "AoI query", "naive scan", "speedup", "vol cut"
    );

    // Same cube cell as the live grid; observer visibility is the 3×3×3 box.
    let cell_of = |p: Position| -> (i32, i32, i32) {
        (
            (p.x / AOI_CELL_SIZE).floor() as i32,
            (p.y / AOI_CELL_SIZE).floor() as i32,
            (p.z / AOI_CELL_SIZE).floor() as i32,
        )
    };

    for &n in &[1_000usize, 5_000, 10_000, 20_000] {
        // Deterministic pseudo-uniform spread across the sector.
        let ships: Vec<(ShipId, Position)> = (0..n)
            .map(|i| {
                let h = |k: usize| ((i.wrapping_mul(k)) % 100_000) as f32 - half;
                (
                    ShipId::new(NodeId(0), i as u64),
                    Position::new(h(2_654_435_761), h(40_503), h(2_246_822_519)),
                )
            })
            .collect();
        let observers: Vec<Position> = ships.iter().step_by(10).map(|(_, p)| *p).collect();
        let p = observers.len();

        // AoI: build one grid, then one neighbor query per observer.
        let t = Instant::now();
        let grid = aoi::CellGrid::build(
            AOI_CELL_SIZE,
            ships
                .iter()
                .map(|(id, p)| (*id, [p.x as f64, p.y as f64, p.z as f64])),
        );
        let build = t.elapsed();
        let t = Instant::now();
        let mut aoi_vol = 0usize;
        for o in &observers {
            aoi_vol += grid
                .neighbors_of([o.x as f64, o.y as f64, o.z as f64])
                .len();
        }
        let query = t.elapsed();

        // Naive (no grid): every observer tests every ship for the same 3×3×3 box.
        let t = Instant::now();
        let mut naive_vol = 0usize;
        for o in &observers {
            let (ox, oy, oz) = cell_of(*o);
            for (_, sp) in &ships {
                let (sx, sy, sz) = cell_of(*sp);
                if (sx - ox).abs() <= 1 && (sy - oy).abs() <= 1 && (sz - oz).abs() <= 1 {
                    naive_vol += 1;
                }
            }
        }
        let scan = t.elapsed();
        assert_eq!(
            aoi_vol, naive_vol,
            "grid and scan must agree on the visible set"
        );

        let no_aoi_vol = p * n; // baseline: every observer receives every ship
        let speedup = scan.as_secs_f64() / query.as_secs_f64().max(1e-9);
        let vol_cut = no_aoi_vol as f64 / aoi_vol.max(1) as f64;
        println!(
            "  {n:>7} {p:>6} | {:>11?} {:>11?} | {:>10?} {:>8.1}x | {:>8.1}x",
            build, query, scan, speedup, vol_cut
        );
    }
    println!();
    println!("  AoI query time tracks local density (k), not n; the no-AoI volume");
    println!("  grows as p·n. This is the lever that raises the TiDi threshold (ADR-0018).");
    println!("═══════════════════════════════════════════");
}
