//! Serve-loop functions: single-node and Raft-cluster WebSocket servers.

mod cluster;
mod single;

pub(crate) use cluster::run_cluster_server;
pub(crate) use single::run_phase4_server;

use crate::{data_loader, ws_server};
use dawn_sector::{aoi, modules, ship_types};
use dawn_sector::node::SimulationNode;
use dawn_sector::spawner::{generate_ships, SpawnConfig};
use dawn_core::{NodeId, SectorBounds, SectorId, ShipId};
use dawn_actor::ClientCommand;

// ── Constants ─────────────────────────────────────────────────────────────────

/// AoI cell edge length (ADR-0019). The sector spans 100,000 units; a 30,000
/// cell gives a 3×3×3 interest region reaching ~30,000–60,000 units from the
/// observer, so most of the sector (including a gate ~49,000 away across a
/// warp) is visible while still culling the far extremes.
pub(crate) const AOI_CELL_SIZE: f32 = 30_000.0;

pub(crate) const P4_SHIPS_DEFAULT: usize = 20;

pub(crate) const P4_TICK_MS: u64 = 100; // 10 Tick/sec

/// Local Time Dilation budget (ADR-0018): logical cost (ship count) a single
/// Sector handles per tick before dilation engages.
pub(crate) const TIDI_BUDGET: f64 = 50_000.0;

// ── DuelMetrics ───────────────────────────────────────────────────────────────

/// Per-ship statistics collected during a duel session.
#[derive(Debug, Default)]
pub(crate) struct ShipDuelStats {
    pub(crate) cap_depletions: u32,
}

/// Session-level metrics for duel mode.
/// Accumulated each tick; printed when ShipDestroyed fires.
#[derive(Debug)]
pub(crate) struct DuelMetrics {
    pub(crate) start_tick: u64,
    /// ship_id → per-ship stats
    pub(crate) stats: std::collections::HashMap<ShipId, ShipDuelStats>,
    /// ShipId of the ship that was destroyed (if duel ended)
    pub(crate) loser: Option<ShipId>,
    /// Tick on which the duel ended
    pub(crate) end_tick: Option<u64>,
}

impl DuelMetrics {
    pub(crate) fn new(start_tick: u64) -> Self {
        Self {
            start_tick,
            stats   : std::collections::HashMap::new(),
            loser   : None,
            end_tick: None,
        }
    }

    pub(crate) fn record_cap_depletions(&mut self, ship_ids: &[ShipId]) {
        for &id in ship_ids {
            self.stats.entry(id).or_default().cap_depletions += 1;
        }
    }

    pub(crate) fn record_end(&mut self, loser: ShipId, tick: u64) {
        self.loser    = Some(loser);
        self.end_tick = Some(tick);
    }

    /// Write a JSON summary to `data/session_<wallclock>.json` for cross-session
    /// balance analysis. Wall-clock timestamp is used only for the filename;
    /// causal ordering relies on the logical Tick (INV-005).
    pub(crate) fn write_json_summary(&self, player_ship_id: Option<ShipId>) {
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

    pub(crate) fn print_summary(&self, player_ship_id: Option<ShipId>) {
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
/// (`--serve`) and clustered (`--serve --cluster`) servers.
///
/// Accepted `LockOn` commands are pushed to `lock_commands` for the tick's
/// Lock System. When `cmd` is a `Jump`, it is returned to the caller so each
/// server can route it appropriately.
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
/// place. Returns `false` if any send fails (the caller drops the session).
pub(crate) fn deliver_aoi_frame(
    sess      : &mut ws_server::PlayerSession,
    node      : &SimulationNode,
    curr      : Vec<ShipId>,
    prev      : &mut Vec<ShipId>,
    new_events: &[dawn_core::DomainEvent],
) -> bool {
    // Ships that have a ShipDestroyed event this tick must NOT receive an
    // AoiLeave — the client's _handle_ship_destroyed already removes them.
    let destroyed_this_tick: std::collections::HashSet<ShipId> = new_events.iter()
        .filter_map(|e| {
            if let dawn_core::DomainEvent::ShipDestroyed(d) = e { Some(d.ship_id) } else { None }
        })
        .collect();

    let old_prev = prev.clone();
    let (entered, left) = aoi::aoi_delta(&old_prev, &curr);
    *prev = curr.clone();

    for id in entered.iter().filter(|&&id| id != sess.ship_id) {
        if let Some(msg) = node.aoi_enter_json(*id) {
            if !sess.conn.send_raw(&msg) { return false; }
        }
    }
    for id in left.iter().filter(|&&id| id != sess.ship_id && !destroyed_this_tick.contains(&id)) {
        if !sess.conn.send_raw(&aoi::aoi_leave_json(*id)) { return false; }
    }

    let visible_events: Vec<_> = new_events.iter()
        .filter(|e| {
            if let dawn_core::DomainEvent::ShipDestroyed(d) = e {
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

/// Build a `SimulationNode` wired the way every serve loop needs it.
/// Shared by `run_phase4_server` and `run_cluster_server`.
pub(crate) fn build_serve_node(id: NodeId, sector: SectorId, bounds: SectorBounds, pop_cap: usize) -> SimulationNode {
    let mut node = SimulationNode::new(id, sector, bounds);
    node.set_population_cap(pop_cap);
    let star_map = data_loader::load_star_map("data/star_map.toml", dawn_sector::galaxy::Galaxy::builtin());
    node.set_galaxy(std::sync::Arc::new(star_map));
    for def in data_loader::load_modules("data/modules.toml", modules::all_modules()) {
        node.register_module(def);
    }
    for def in data_loader::load_ship_types("data/ship_types.toml", ship_types::all_ship_types()) {
        node.register_ship_type(def);
    }
    node
}

/// Spawn `ship_count` NPC frigates into `node`, each fitted with a small railgun.
/// Shared by `run_phase4_server` and `run_cluster_server`.
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
    /// through the `dyn ClientConnection` seam (ADR-0005) with no socket.
    #[test]
    fn player_move_over_connection_is_applied_and_velocity_event_flows_back() {
        let bounds   = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
        let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, node::POPULATION_CAP);

        let player_id = node.next_player_id();
        let ship_id   = node.spawn_player_ship_at_pub(player_id, Position::new(0.0, 0.0, 0.0));

        let (mut server, client) = InProcessConnection::pair();

        client.command_tx.send(ClientCommand::Move(MoveCommand {
            ship_id,
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
