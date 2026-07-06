//! Single-node WebSocket server (`--serve`, no Raft cluster).

use super::{build_serve_node, AoiDelivery, DuelMetrics, AOI_CELL_SIZE, P4_TICK_MS, TIDI_BUDGET};
use crate::ws_server;
use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, ShipId};
use dawn_sector::dilation;
use dawn_sector::node::ClientCommandFollowup;
use tokio::sync::mpsc;

pub(crate) async fn run_phase4_server(
    ship_count: usize,
    duel_mode: bool,
    enemy_count: usize,
    pop_cap: usize,
) {
    println!("═══════════════════════════════════════════");
    println!("  Phase 5 — Godot WebSocket server          ");
    println!("═══════════════════════════════════════════");
    if duel_mode {
        println!("  mode: DUEL (1 human vs {enemy_count} Bot(s), no NPC)");
    } else {
        println!("  npc ships: {ship_count}  (change with --ships N)");
    }
    println!(
        "  tick rate: {} ms/tick  ({} tick/sec)",
        P4_TICK_MS,
        1000 / P4_TICK_MS
    );
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = ws_server::WsServer::bind("127.0.0.1:7878")
        .await
        .expect("failed to bind WebSocket server");

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, pop_cap);

    node.spawn_npc_frigates(ship_count);
    // Duel-mode player spawn: close enough to the Bot to be within weapon
    // range (Small Railgun: 3000 range + 2000 falloff = 5000) from the
    // moment the human connects, instead of the universe-wide
    // DEFAULT_PLAYER_SPAWN (30_000 units away -- so far from the Bot's
    // fixed spawn that every weapon activation was rejected as out-of-range
    // immediately after Lock, which looked to the player like the turret
    // flickering on then instantly off).
    let duel_player_spawn = Position::new(1200.0 + 2000.0, 0.0, 0.0);
    if duel_mode {
        // Spread multiple enemy Bots along +Y so they don't spawn stacked on
        // top of each other, while staying within the player's weapon range
        // (Small Railgun: 3000 + 2000 falloff = 5000) for --enemies N>1 too
        // (e.g. to practice locking/engaging more than one target at once).
        for i in 0..enemy_count.max(1) {
            let bot_pos = Position::new(1200.0, i as f32 * 800.0, 0.0);
            let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);
            println!(
                "  [Server] Duel mode: Bot ship #{} ready at {:?}",
                bot_ship_id.raw(),
                bot_pos
            );
        }
    }

    println!("  [Server] {ship_count} NPC ships ready. Waiting for players...");

    let (new_conn_tx, mut new_conn_rx) =
        mpsc::unbounded_channel::<(tokio::net::TcpStream, std::net::SocketAddr)>();
    let (ready_sess_tx, mut ready_sess_rx) = mpsc::unbounded_channel::<ws_server::PlayerSession>();

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
    let mut aoi_delivery = AoiDelivery::new(AOI_CELL_SIZE);
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(P4_TICK_MS));

    let mut duel_metrics: Option<DuelMetrics> = None;
    let mut player_ship_id: Option<ShipId> = None;
    let mut tidi = dilation::DilationController::new(TIDI_BUDGET);

    loop {
        interval.tick().await;

        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            if node.at_population_cap() {
                eprintln!(
                    "[Server] connection from {addr} refused: Sector at population cap ({} ships)",
                    node.ship_count()
                );
                drop(stream);
                continue;
            }
            let player_id = node.next_player_id();
            let ship_id = if duel_mode {
                node.spawn_player_ship_at_pub(player_id, duel_player_spawn)
            } else {
                node.spawn_player_ship(player_id)
            };
            let payload = node.build_handoff_payload(ship_id, AOI_CELL_SIZE);
            let tx = ready_sess_tx.clone();

            if duel_mode && player_ship_id.is_none() {
                player_ship_id = Some(ship_id);
                let tick = node.current_tick().value();
                duel_metrics = Some(DuelMetrics::new(tick));
                println!("  [Duel] metrics collection started at tick {tick}");
            }

            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream,
                    addr,
                    player_id,
                    ship_id,
                    &payload.initial_state,
                    payload.player_loadout,
                )
                .await
                {
                    Ok(sess) => {
                        let _ = tx.send(sess);
                    }
                    Err(e) => eprintln!("[Server] handshake failed: {e}"),
                }
            });
        }

        while let Ok(sess) = ready_sess_rx.try_recv() {
            println!(
                "  [Server] {} joined with ship #{}",
                sess.player_id,
                sess.ship_id.raw()
            );
            let seed = node
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| node.ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            aoi_delivery.seed_player(sess.player_id, seed);
            sessions.push(sess);
        }

        let events_before: u64 = node.total_event_count() as u64;

        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();
        for sess in sessions.iter_mut() {
            while let Some(cmd) = sess.try_recv_command() {
                match node.apply_client_command(sess.player_id, cmd, &mut lock_commands) {
                    Some(ClientCommandFollowup::Jump(j)) => {
                        eprintln!(
                            "[Server] JumpCommand ignored (ship #{} gate #{}): \
                             --serve runs a single-sector node without Raft",
                            j.ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                    Some(ClientCommandFollowup::RefreshFitting(ship_id)) => {
                        if let Some(json) = node.build_player_loadout_json(ship_id) {
                            sess.send_raw(&json);
                        }
                    }
                    None => {}
                }
            }
        }

        let tick_result = node.tick_with_lock_commands(&lock_commands);

        for sess in &sessions {
            let should_refresh = tick_result.events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::ShipDestroyed(destroyed)
                        if destroyed.killer_id == sess.ship_id
                )
            });
            if should_refresh {
                if let Some(json) = node.build_player_loadout_json(sess.ship_id) {
                    sess.send_raw(&json);
                }
            }
        }

        if duel_mode {
            if let Some(ref mut metrics) = duel_metrics {
                metrics.record_cap_depletions(&tick_result.cap_depletions);

                for event in &tick_result.events {
                    if let dawn_core::DomainEvent::ShipDestroyed(e) = event {
                        metrics.record_end(e.ship_id, tick_result.tick.value());
                        metrics.print_summary(player_ship_id);
                        metrics.write_json_summary(player_ship_id);
                    }
                }
            }
        }

        let all_new_events: Vec<_> = {
            use dawn_event_store::store::EventStore as _;
            node.event_store()
                .iter_from(events_before)
                .map(|r| r.event.clone())
                .collect()
        };
        let warp_arrivals = node.drain_completed_warps();
        aoi_delivery.deliver_single_sector(&node, &mut sessions, &all_new_events, &warp_arrivals);

        let was_dilated = tidi.is_dilated();
        let prior_active = tidi.active_ticks();
        tidi.update(node.ship_count() as f64);
        if tidi.is_dilated() {
            if !was_dilated {
                println!(
                    "[TiDi] dilation engaged: factor={:.2} (cost {} > budget {TIDI_BUDGET:.0})",
                    tidi.dilation(),
                    node.ship_count()
                );
            }
            let extra = tidi.paced_tick_ms(P4_TICK_MS as f64) - P4_TICK_MS as f64;
            tokio::time::sleep(std::time::Duration::from_millis(extra as u64)).await;
        } else if was_dilated {
            println!("[TiDi] dilation recovered to real-time after {prior_active} ticks (cost {} <= budget {TIDI_BUDGET:.0})",
                node.ship_count());
        }
    }
}
