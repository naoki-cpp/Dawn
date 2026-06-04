//! Simulation entry point.
//!
//! Phase 1: single-node benchmark  (10,000 ships × 100 ticks)
//! Phase 2: multi-node demo        (3 nodes × 1,000 ships × 20 ticks)
//!
//! Usage:
//!   cargo run -p dawn-simulation --bin simulate
//!   cargo run -p dawn-simulation --bin simulate --release

mod cluster;
mod node;
mod sector_simulator_actor;
mod spawner;

use dawn_core::{NodeId, SectorBounds, SectorId};
use node::SimulationNode;
use spawner::{generate_ships, SpawnConfig};
use std::time::Instant;

use cluster::MultiNodeCluster;

// ── Constants ─────────────────────────────────────────────────────────────────

const P1_SHIPS : usize = 10_000;
const P1_TICKS : usize = 100;

const P2_NODES : usize = 3;
const P2_SHIPS : usize = 1_000;   // per node
const P2_TICKS : usize = 20;

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    run_phase1_benchmark();
    println!();
    run_phase2_demo().await;
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
        node.spawn_ship(pos, vel);
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
