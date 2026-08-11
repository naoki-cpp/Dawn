//! Simulation entry point.
//!
//! Phase 1: single-node benchmark  (10,000 ships × 100 ticks)
//! Phase 2: multi-node demo        (3 nodes × 1,000 ships × 20 ticks)
//!
//! Usage:
//!   cargo run -p dawn-server --bin simulate
//!   cargo run -p dawn-server --bin simulate --release

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

mod bench;
mod cluster;
mod runtime_frame;
mod sector_runtime_driver;
mod serve;

// Client transport (WsServer) is shared via dawn-actor; bring the module into
// crate scope so existing `crate::ws_server` paths keep resolving.
use dawn_actor::ws_server;

#[cfg(test)]
fn test_catalog() -> std::sync::Arc<dawn_sector::game_data::GameDataCatalog> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::sync::Arc::new(
        dawn_sector::game_data::GameDataCatalog::load_from_paths(
            root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
            root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog"),
    )
}

#[tokio::main]
async fn main() {
    // Phase 4 mode: with --serve, start the Godot-facing WebSocket server
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--serve".to_string()) {
        // --duel: 1 human vs 1 Bot, no NPC ships
        let duel_mode = args.contains(&"--duel".to_string());
        let usize_arg = |name: &str| {
            args.windows(2)
                .find(|w| w[0] == name)
                .and_then(|w| w[1].parse::<usize>().ok())
        };
        // --ships N sets the NPC count (ignored in --duel mode)
        let ship_count = if duel_mode {
            0
        } else {
            usize_arg("--ships").unwrap_or(serve::P4_SHIPS_DEFAULT)
        };
        // --pop-cap N sets the per-Sector population backstop (ADR-0018, last
        // resort); set it low to observe new connections being refused.
        let pop_cap = usize_arg("--pop-cap").unwrap_or(dawn_sector::node::POPULATION_CAP);
        // --enemies N sets the Bot count in --duel mode (default 1) -- e.g.
        // to practice locking/engaging more than one target, or Remote
        // Repair against a Locked target, without needing a real ally.
        let enemy_count = usize_arg("--enemies").unwrap_or(1);
        // --cluster: 3-node Raft cluster so Jump Gates work (ADR-0009)
        if args.contains(&"--cluster".to_string()) {
            serve::run_cluster_server(ship_count, pop_cap).await;
            return;
        }
        serve::run_phase4_server(ship_count, duel_mode, enemy_count, pop_cap).await;
        return;
    }

    // Phase 7 mode: --raft-demo runs an observable 3-node Raft Transit demo
    // (leader election, Transit through the Raft Log, leader failover).
    if args.contains(&"--raft-demo".to_string()) {
        bench::run_raft_demo().await;
        return;
    }

    // AoI scaling benchmark (ADR-0019): show the static cell grid keeps the
    // per-tick interest cost and delivery volume bounded as ships/players grow.
    if args.contains(&"--aoi-bench".to_string()) {
        bench::run_aoi_benchmark();
        return;
    }

    bench::run_phase1_benchmark();
    println!();
    bench::run_phase2_demo().await;
    println!();
    bench::run_phase3_demo();
}
