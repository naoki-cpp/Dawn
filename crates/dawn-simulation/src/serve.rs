//! Serve-loop functions: single-node and Raft-cluster WebSocket servers.

use crate::{cluster, data_loader, ws_server};
use dawn_sector::{aoi, dilation, modules, ship_types, transit};
use dawn_sector::node::SimulationNode;
use dawn_sector::spawner::{generate_ships, SpawnConfig};
use dawn_actor::ClientCommand;
use dawn_core::{
    DomainEvent, NodeId, PlayerId, Position,
    SectorBounds, SectorId, ShipId, WarpTarget,
};
use std::collections::HashMap;
use tokio::sync::mpsc;
use ws_server::WsServer;

// ── Constants ─────────────────────────────────────────────────────────────────

/// AoI cell edge length (ADR-0019). The sector spans 100,000 units; a 30,000
/// cell gives a 3×3×3 interest region reaching ~30,000–60,000 units from the
/// observer, so most of the sector (including a gate ~49,000 away across a
/// warp) is visible while still culling the far extremes. This is a scalability
/// knob: shrink it for dense large-scale tests. Tuned in benches later.
pub(crate) const AOI_CELL_SIZE: f32 = 30_000.0;

pub(crate) const P4_SHIPS_DEFAULT: usize = 20;

pub(crate) const P4_TICK_MS: u64 = 100; // 10 Tick/sec

/// Local Time Dilation budget (ADR-0018): logical cost (ship count) a single
/// Sector handles per tick before dilation engages. Set far above normal load —
/// the anti-TiDi threshold is intentionally very high (only extreme density).
pub(crate) const TIDI_BUDGET: f64 = 50_000.0;

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
    start_tick: u64,
    /// ship_id → per-ship stats
    stats: HashMap<ShipId, ShipDuelStats>,
    /// ShipId of the ship that was destroyed (if duel ended)
    loser: Option<ShipId>,
    /// Tick on which the duel ended
    end_tick: Option<u64>,
}

impl DuelMetrics {
    fn new(start_tick: u64) -> Self {
        Self {
            start_tick,
            stats   : HashMap::new(),
            loser   : None,
            end_tick: None,
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

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Apply a player command that behaves identically in the single-node
/// (`--serve`) and clustered (`--serve --cluster`) servers — every command
/// except `Jump`, whose handling differs (ignored vs. proposed to Raft).
///
/// Accepted `LockOn` commands are pushed to `lock_commands` for the tick's
/// Lock System. When `cmd` is a `Jump`, it is returned to the caller so each
/// server can route it appropriately instead of being handled here.
pub(crate) fn apply_common_command(
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
            if node.owns_ship(player_id, lo.ship_id) {
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
        // Warp: intra-Sector short-range Fold toward a gate (ADR-0022).
        ClientCommand::Warp(w) => { node.apply_warp_command_owned(player_id, w); }
        // Jump differs per server: hand it back to the caller.
        ClientCommand::Jump(j) => return Some(j),
    }
    None
}

/// Push one Area-of-Interest frame to a session (ADR-0019): `AoiEnter` for ships
/// that just became visible, `AoiLeave` for ships that left, then the new domain
/// events that concern a currently-visible ship. `prev` is updated to `curr` in
/// place. Returns `false` if any send fails (the caller drops the session, which
/// also clears its stale `prev`). Shared by the single-node and clustered loops.
pub(crate) fn deliver_aoi_frame(
    sess      : &mut ws_server::PlayerSession,
    node      : &SimulationNode,
    curr      : Vec<ShipId>,
    prev      : &mut Vec<ShipId>,
    new_events: &[dawn_core::DomainEvent],
) -> bool {
    // Ships that have a ShipDestroyed event this tick must NOT receive an
    // AoiLeave — the client's _handle_ship_destroyed already removes them,
    // plays the explosion, and clears the lock target. Sending AoiLeave first
    // would remove the ship from _ships before ShipDestroyed arrives, making
    // the destruction handler a no-op (early return on `!_ships.has(id)`).
    let destroyed_this_tick: std::collections::HashSet<ShipId> = new_events.iter()
        .filter_map(|e| {
            if let dawn_core::DomainEvent::ShipDestroyed(d) = e { Some(d.ship_id) } else { None }
        })
        .collect();

    let old_prev = prev.clone();
    let (entered, left) = aoi::aoi_delta(&old_prev, &curr);
    *prev = curr.clone();

    // Tell the client about ships entering / leaving its neighborhood.
    for id in entered.iter().filter(|&&id| id != sess.ship_id) {
        if let Some(msg) = node.aoi_enter_json(*id) {
            if !sess.conn.send_raw(&msg) { return false; }
        }
    }
    // Skip AoiLeave for destroyed ships — ShipDestroyed handles client removal.
    for id in left.iter().filter(|&&id| id != sess.ship_id && !destroyed_this_tick.contains(&id)) {
        if !sess.conn.send_raw(&aoi::aoi_leave_json(*id)) { return false; }
    }

    // Deliver only events concerning a currently-visible ship.
    // For ShipDestroyed, use the pre-tick visible set (old_prev) so the event
    // reaches the client even though the ship was removed from the grid this tick.
    let visible_events: Vec<_> = new_events.iter()
        .filter(|e| {
            if let dawn_core::DomainEvent::ShipDestroyed(d) = e {
                // Was the destroyed ship (or its killer) visible before this tick?
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

/// Build a `SimulationNode` wired the way every serve loop needs it: the
/// population backstop set (ADR-0018) and the module / ship-type registries
/// loaded from `data/*.toml` (falling back to the built-in defaults). Shared
/// by `run_phase4_server` and `run_cluster_server` so the wiring cannot drift.
pub(crate) fn build_serve_node(id: NodeId, sector: SectorId, bounds: SectorBounds, pop_cap: usize) -> SimulationNode {
    let mut node = SimulationNode::new(id, sector, bounds);
    node.set_population_cap(pop_cap);
    let star_map = data_loader::load_star_map("data/star_map.toml", dawn_sector::star_map::StarMap::builtin());
    node.set_star_map(std::sync::Arc::new(star_map));
    for def in data_loader::load_modules("data/modules.toml", modules::all_modules()) {
        node.register_module(def);
    }
    for def in data_loader::load_ship_types("data/ship_types.toml", ship_types::all_ship_types()) {
        node.register_ship_type(def);
    }
    node
}

/// Spawn `ship_count` NPC frigates into `node`, each fitted with a small
/// railgun. Shared by `run_phase4_server` and `run_cluster_server`.
pub(crate) fn spawn_npc_frigates(node: &mut SimulationNode, ship_count: usize) {
    let config = SpawnConfig::default_for_node(NodeId(0));
    for (_, pos, vel) in generate_ships(ship_count, &config, 0) {
        let ship_id = node.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
        node.fit_module(dawn_core::FitModuleCommand {
            ship_id,
            slot     : dawn_core::SlotKind::High,
            module_id: modules::MODULE_RAILGUN_SMALL,
        });
    }
}

// ── Phase 5: Godot WebSocket server (multi-client) ────────────────────────────

pub(crate) async fn run_phase4_server(ship_count: usize, duel_mode: bool, pop_cap: usize) {
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
    let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, pop_cap);

    // Generate NPC ships
    spawn_npc_frigates(&mut node, ship_count);
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
    // AoI: each session's last-known visible set (ADR-0019), for enter/leave diffs.
    let mut prev_visible: std::collections::HashMap<dawn_core::PlayerId, Vec<dawn_core::ShipId>> =
        std::collections::HashMap::new();
    let mut interval = tokio::time::interval(
        std::time::Duration::from_millis(P4_TICK_MS)
    );

    // Duel mode: track session metrics (None until the player connects).
    let mut duel_metrics  : Option<DuelMetrics> = None;
    let mut player_ship_id: Option<ShipId>      = None;
    // Local Time Dilation (ADR-0018). One Sector here, so dilation is inherently
    // local. The budget is set far above normal load (anti-TiDi threshold), so it
    // only engages under extreme density and auto-recovers.
    let mut tidi = dilation::DilationController::new(TIDI_BUDGET);

    loop {
        interval.tick().await;

        // New TCP connection → spawn handshake task.
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            // Population backstop (ADR-0018, last resort): refuse new entrants
            // when the Sector is already at its ship-count cap. Set far above
            // the TiDi budget, so split/LoD/local-TiDi engage well before this.
            if node.at_population_cap() {
                eprintln!("[Server] connection from {addr} refused: Sector at population cap ({} ships)",
                    node.ship_count());
                drop(stream);
                continue;
            }
            let player_id      = node.next_player_id();
            let ship_id        = node.spawn_player_ship(player_id);
            // AoI: send only the ships in the new player's 27-cell neighborhood
            // (ADR-0019). Falls back to full state if the position is unavailable.
            let initial_state  = match node.get_ship_position(ship_id) {
                Some(pos) => node.build_initial_state_json_for(pos, AOI_CELL_SIZE),
                None      => node.build_initial_state_json(),
            };
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
            // Seed the AoI baseline with the set already sent in InitialState, so
            // the first tick's enter/leave diff does not re-send those ships.
            let seed = node.get_ship_position(sess.ship_id)
                .map(|pos| node.ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            prev_visible.insert(sess.player_id, seed);
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

        // Broadcast new events (command-driven + tick) to clients, filtered by
        // each session's Area of Interest (ADR-0019).
        let all_new_events: Vec<_> = {
            use dawn_event_store::store::EventStore as _;
            node.event_store().iter_from(events_before)
                .map(|r| r.event.clone())
                .collect()
        };
        let grid = aoi::CellGrid::build(AOI_CELL_SIZE, node.ship_positions());
        sessions.retain_mut(|sess| {
            let curr = node.get_ship_position(sess.ship_id)
                .map(|pos| grid.neighbors_of(pos))
                .unwrap_or_default();
            let prev = prev_visible.entry(sess.player_id).or_default();
            deliver_aoi_frame(sess, &node, curr, prev, &all_new_events)
        });
        // Drop AoI state for sessions that disconnected this tick.
        prev_visible.retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));

        // Local Time Dilation (ADR-0018): the decision uses a logical cost
        // (ship count), never wall-clock, and only stretches real time — the
        // tick above already ran unchanged. Engage/recover is logged for SLA.
        let was_dilated = tidi.is_dilated();
        let prior_active = tidi.active_ticks();
        tidi.update(node.ship_count() as f64);
        if tidi.is_dilated() {
            if !was_dilated {
                println!("[TiDi] dilation engaged: factor={:.2} (cost {} > budget {TIDI_BUDGET:.0})",
                    tidi.dilation(), node.ship_count());
            }
            let extra = tidi.paced_tick_ms(P4_TICK_MS as f64) - P4_TICK_MS as f64;
            tokio::time::sleep(std::time::Duration::from_millis(extra as u64)).await;
        } else if was_dilated {
            println!("[TiDi] dilation recovered to real-time after {prior_active} ticks (cost {} <= budget {TIDI_BUDGET:.0})",
                node.ship_count());
        }
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
/// - Players spawn at the Sector origin, far from Gate 0, so the intended
///   travel loop is demonstrable: warp (W) or approach (A) to the gate, then
///   jump (J) once in range (ADR-0022).
pub(crate) async fn run_cluster_server(ship_count: usize, pop_cap: usize) {
    use dawn_event_store::store::EventStore as _;
    use transit::TransitOp;

    const SECTORS: usize = 3;
    /// Sector origin: ~49,000u from Gate 0 (x=49,000), well beyond the 3,000u
    /// warp minimum, so warp/approach to the gate both work (ADR-0022).
    const PLAYER_SPAWN: Position = Position { x: 0.0, y: 0.0, z: 0.0 };

    println!("═══════════════════════════════════════════");
    println!("  Phase 7.5 — Raft cluster WebSocket server ");
    println!("═══════════════════════════════════════════");
    println!("  sectors  : {SECTORS} (one Raft node each)");
    println!("  npc ships: {ship_count} in Sector 0  (change with --ships N)");
    println!("  tick rate: {} ms/tick  ({} tick/sec)", P4_TICK_MS, 1000 / P4_TICK_MS);
    println!("  travel   : select Gate 0 (click its ring), press W to warp (or A to approach),");
    println!("             then J to jump once in range (player spawns at the Sector origin)");
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
    let mut nodes: Vec<SimulationNode> = ids.iter()
        .map(|&id| build_serve_node(id, SectorId(id.0), bounds, pop_cap))
        .collect();

    // NPC ships live in Sector 0 only.
    spawn_npc_frigates(&mut nodes[0], ship_count);

    // Warm up: tick until a Raft leader is elected so early Jump proposals
    // are not dropped (election timeout is at most 20 ticks).
    for _ in 0..30 {
        for i in 0..SECTORS {
            transit::step_cluster_node(&mut nodes[i], &rafts[i], &mut committed_rxs[i], &[]);
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
    // AoI: each session's last-known visible set (ADR-0019), for enter/leave diffs.
    let mut prev_visible : HashMap<PlayerId, Vec<ShipId>> = HashMap::new();

    let mut interval = tokio::time::interval(
        std::time::Duration::from_millis(P4_TICK_MS)
    );

    loop {
        interval.tick().await;

        // New TCP connection → spawn player in Sector 0 near Gate 0.
        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            // Population backstop (ADR-0018, last resort): refuse new entrants
            // when Sector 0 is already at its ship-count cap.
            if nodes[0].at_population_cap() {
                eprintln!("[Server] connection from {addr} refused: Sector 0 at population cap ({} ships)",
                    nodes[0].ship_count());
                drop(stream);
                continue;
            }
            let player_id      = nodes[0].next_player_id();
            let ship_id        = nodes[0].spawn_player_ship_at_pub(player_id, PLAYER_SPAWN);
            // AoI: scope the connect-time state to the player's 27-cell neighborhood (ADR-0019).
            let initial_state  = match nodes[0].get_ship_position(ship_id) {
                Some(pos) => nodes[0].build_initial_state_json_for(pos, AOI_CELL_SIZE),
                None      => nodes[0].build_initial_state_json(),
            };
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
            // Seed the AoI baseline (sector 0) with the InitialState set.
            let seed = nodes[0].get_ship_position(sess.ship_id)
                .map(|pos| nodes[0].ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            prev_visible.insert(sess.player_id, seed);
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
                let ship_owned = j.ship_id == sess.ship_id;
                let in_range   = ship_owned && nodes[sector].can_propose_jump(j.ship_id, j.gate_id);
                if in_range {
                    let to = nodes[sector].jump_gate(j.gate_id)
                        .expect("can_propose_jump confirmed gate exists")
                        .to_sector;
                    rafts[sector].propose(
                        TransitOp::Request { ship_id: j.ship_id, to, gate_id: Some(j.gate_id) }.encode(),
                    );
                    println!("  [Server] Jump proposed: ship #{} gate #{} (S{} → S{})",
                        j.ship_id.raw(), j.gate_id.0, sector, to.0);
                } else if ship_owned && nodes[sector].apply_warp_command(j.ship_id, WarpTarget::Gate(j.gate_id), true) {
                    // Ship is out of range: warp to the gate first; the node will
                    // queue a pending_auto_jump once warp completes (ADR-0022).
                    println!("  [Server] Jump: ship #{} out of range — auto-warp to gate #{} started",
                        j.ship_id.raw(), j.gate_id.0);
                } else {
                    eprintln!("[Server] JumpCommand rejected (ship #{} gate #{})",
                        j.ship_id.raw(), j.gate_id.0);
                }
            }
        }

        // Tick every node in the canonical step order (Step 7.5 → sim → Step 10).
        for i in 0..SECTORS {
            transit::step_cluster_node(&mut nodes[i], &rafts[i], &mut committed_rxs[i], &lock_commands[i]);
        }

        // Propose auto-jumps triggered by warp completion this tick.
        for i in 0..SECTORS {
            for (ship_id, gate_id) in nodes[i].drain_pending_auto_jumps() {
                if nodes[i].can_propose_jump(ship_id, gate_id) {
                    let to = nodes[i].jump_gate(gate_id)
                        .expect("gate must exist if can_propose_jump passed")
                        .to_sector;
                    rafts[i].propose(
                        TransitOp::Request { ship_id, to, gate_id: Some(gate_id) }.encode(),
                    );
                    println!("  [Server] Auto-jump proposed: ship #{} gate #{} (S{} → S{})",
                        ship_id.raw(), gate_id.0, i, to.0);
                }
            }
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
        // JumpGateUsed / StarSystemChanged for the jumping ship's own player,
        // collected here because jumped sessions skip deliver_aoi_frame below
        // (their old Sector's AoI is no longer relevant) and would otherwise
        // never receive these events — leaving the client's system display stale.
        let mut jump_own_events: HashMap<PlayerId, Vec<DomainEvent>> = HashMap::new();
        for sector_events in &events_by_sector {
            for event in sector_events {
                match event {
                    DomainEvent::JumpGateUsed(e) => {
                        if let Some(&player_id) = ship_player.get(&e.ship_id) {
                            let dest = e.to_sector.0 as usize;
                            nodes[dest].adopt_player_ship(e.ship_id, player_id);
                            player_sector.insert(player_id, dest);
                            jumped_players.push((player_id, dest));
                            jump_own_events.entry(player_id).or_default().push(event.clone());
                            println!("  [Server] {player_id:?} ship #{} now owned by Sector {dest}",
                                e.ship_id.raw());
                        }
                    }
                    DomainEvent::StarSystemChanged(e) => {
                        if let Some(&player_id) = ship_player.get(&e.ship_id) {
                            jump_own_events.entry(player_id).or_default().push(event.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Send each session only the events from the Sector it occupies, further
        // filtered by its Area of Interest (ADR-0019). One grid per Sector.
        let grids: Vec<aoi::CellGrid> = nodes.iter()
            .map(|n| aoi::CellGrid::build(AOI_CELL_SIZE, n.ship_positions()))
            .collect();
        let jumped_ids: std::collections::HashSet<PlayerId> =
            jumped_players.iter().map(|(p, _)| *p).collect();

        sessions.retain_mut(|sess| {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            let curr = nodes[sector].get_ship_position(sess.ship_id)
                .map(|pos| grids[sector].neighbors_of(pos))
                .unwrap_or_default();

            // A player that just jumped gets a fresh scoped InitialState below;
            // reset its AoI baseline and skip enter/leave this tick.
            if jumped_ids.contains(&sess.player_id) {
                prev_visible.insert(sess.player_id, curr);
                return true;
            }

            let prev = prev_visible.entry(sess.player_id).or_default();
            deliver_aoi_frame(sess, &nodes[sector], curr, prev, &events_by_sector[sector])
        });
        prev_visible.retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));

        // Resend scoped InitialState to players that just jumped so the client
        // clears the old Sector's ships (and its stale lock) and rebuilds from
        // the destination Sector. `_on_initial_state` resets `_player_lock_target`.
        for (player_id, dest) in jumped_players {
            if let Some(sess) = sessions.iter().find(|s| s.player_id == player_id) {
                if let Some(events) = jump_own_events.get(&player_id) {
                    sess.send_events(events);
                }
                let initial_state = nodes[dest].get_ship_position(sess.ship_id)
                    .map(|pos| nodes[dest].build_initial_state_json_for(pos, AOI_CELL_SIZE))
                    .unwrap_or_else(|| nodes[dest].build_initial_state_json());
                sess.conn.send_raw(&initial_state);
            }
        }
    }
}

// ── Integration tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod serve_pipeline_tests {
    use super::*;
    use dawn_sector::node;
    use dawn_actor::{ClientCommand, ClientConnection, InProcessConnection};
    use dawn_core::{DomainEvent, MoveCommand, NodeId, Position, SectorBounds, SectorId};
    use dawn_event_store::store::EventStore as _;

    /// A player's `Move`, delivered over a `ClientConnection`, is applied to the
    /// owning node and the resulting `VelocityChanged` is delivered back over the
    /// same connection. Exercises the command → tick → event serve pipeline
    /// through the `dyn ClientConnection` seam (ADR-0005) with no socket — the
    /// in-process path the WebSocket loop mirrors.
    #[test]
    fn player_move_over_connection_is_applied_and_velocity_event_flows_back() {
        let bounds   = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
        let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, node::POPULATION_CAP);

        let player_id = node.next_player_id();
        let ship_id   = node.spawn_player_ship_at_pub(player_id, Position::new(0.0, 0.0, 0.0));

        let (mut server, client) = InProcessConnection::pair();

        // Client requests thrust toward +X.
        client.command_tx.send(ClientCommand::Move(MoveCommand {
            ship_id,
            target_position: Position::new(1_000.0, 0.0, 0.0),
        })).expect("server connection is alive");

        // Server side: drain and apply commands purely through the trait object.
        let conn: &mut dyn ClientConnection = &mut server;
        let mut lock_commands = Vec::new();
        let before = node.total_event_count() as u64;
        while let Some(cmd) = conn.try_recv_command() {
            apply_common_command(&mut node, player_id, cmd, &mut lock_commands);
        }

        node.tick_with_lock_commands(&lock_commands);

        // Deliver the tick's new events back over the connection.
        let new_events: Vec<DomainEvent> = node
            .event_store()
            .iter_from(before)
            .map(|r| r.event.clone())
            .collect();
        conn.send_events(&new_events).expect("client endpoint is alive");

        // Client observes a VelocityChanged for its own ship.
        let mut client = client;
        let mut saw_velocity_changed = false;
        while let Ok(ev) = client.event_rx.try_recv() {
            if let DomainEvent::VelocityChanged(vc) = ev {
                if vc.ship_id == ship_id {
                    saw_velocity_changed = true;
                }
            }
        }
        assert!(
            saw_velocity_changed,
            "client must receive a VelocityChanged for the ship it moved"
        );
    }

    /// A command for a ship the player does not own is rejected by the pipeline:
    /// no `VelocityChanged` reaches the client. Guards the ownership check in
    /// `apply_common_command` → `apply_move_command_owned` (CLAUDE.md §5).
    #[test]
    fn move_for_unowned_ship_produces_no_event_over_connection() {
        let bounds   = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
        let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, node::POPULATION_CAP);

        let player_id = node.next_player_id();
        let _own_ship = node.spawn_player_ship_at_pub(player_id, Position::new(0.0, 0.0, 0.0));
        // A second player's ship the first player must not be able to drive.
        let other_id   = node.next_player_id();
        let other_ship = node.spawn_player_ship_at_pub(other_id, Position::new(500.0, 0.0, 0.0));

        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(ClientCommand::Move(MoveCommand {
            ship_id: other_ship,
            target_position: Position::new(1_000.0, 0.0, 0.0),
        })).expect("server connection is alive");

        let conn: &mut dyn ClientConnection = &mut server;
        let mut lock_commands = Vec::new();
        let before = node.total_event_count() as u64;
        while let Some(cmd) = conn.try_recv_command() {
            apply_common_command(&mut node, player_id, cmd, &mut lock_commands);
        }
        node.tick_with_lock_commands(&lock_commands);

        let new_events: Vec<DomainEvent> = node
            .event_store()
            .iter_from(before)
            .map(|r| r.event.clone())
            .collect();
        conn.send_events(&new_events).expect("client endpoint is alive");

        let mut client = client;
        while let Ok(ev) = client.event_rx.try_recv() {
            if let DomainEvent::VelocityChanged(vc) = ev {
                assert_ne!(
                    vc.ship_id, other_ship,
                    "a player must not be able to move a ship it does not own"
                );
            }
        }
    }
}
