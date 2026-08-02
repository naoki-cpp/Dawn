//! Single-node WebSocket server (`--serve`, no Raft cluster).

use super::{
    build_serve_node, market::MarketRuntime, AoiDelivery, DuelMetrics, AOI_CELL_SIZE, P4_TICK_MS,
    TIDI_BUDGET,
};
use crate::ws_server;
use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal,
    CommittedClientAdmission,
};
use dawn_sector::dilation;
use dawn_sector::node::{ClientCommandFollowup, SimulationNode};
use dawn_wire::ServerMessage;
use tokio::sync::mpsc;

type HandshakeCompletion = (
    ClientAdmissionAttempt,
    Result<ws_server::PlayerSession, String>,
);

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
    let mut market = MarketRuntime::open("data/market.sqlite")
        .expect("failed to open Market database at data/market.sqlite");

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
            let bot_pos = Position::new(1200.0, i as f64 * 800.0, 0.0);
            let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);
            println!(
                "  [Server] Duel mode: Bot ship #{} ready at {:?}",
                bot_ship_id.raw(),
                bot_pos
            );
        }
    }

    println!("  [Server] {ship_count} NPC ships ready. Waiting for players...");

    let (handshake_req_tx, mut handshake_req_rx) =
        mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel::<HandshakeCompletion>();

    let server_arc = std::sync::Arc::new(server);
    let server_clone = server_arc.clone();
    tokio::spawn(async move {
        loop {
            if let Some((stream, addr)) = server_clone.try_accept_raw().await {
                let tx = handshake_req_tx.clone();
                tokio::spawn(async move {
                    match ws_server::WsServer::accept_handshake_request(stream, addr).await {
                        Ok(request) => {
                            let _ = tx.send(request);
                        }
                        Err(error) => eprintln!("[Server] handshake request failed: {error}"),
                    }
                });
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

        // Resolve socket outcomes on the tick-loop thread. A session is not
        // visible to AoI delivery or command routing until its Sector attempt
        // commits successfully.
        while let Ok((attempt, result)) = completion_rx.try_recv() {
            if let Some((sess, committed)) = finish_single_admission(&mut node, attempt, result) {
                send_post_commit_loadout(&node, &sess, committed);
                println!(
                    "  [Server] {} joined with ship #{}",
                    sess.player_id,
                    sess.ship_id.raw()
                );
                aoi_delivery.seed_single_player(&node, sess.player_id, sess.ship_id);

                if duel_mode && player_ship_id.is_none() {
                    player_ship_id = Some(sess.ship_id);
                    let tick = node.current_tick().value();
                    duel_metrics = Some(DuelMetrics::new(tick));
                    println!("  [Duel] metrics collection started at tick {tick}");
                }

                sessions.push(sess);
            }
        }

        while let Ok(request) = handshake_req_rx.try_recv() {
            let spawn_position = if duel_mode {
                duel_player_spawn
            } else {
                Position::new(30_000.0, 0.0, 0.0)
            };
            let intent = match request.resume {
                Some(resume) => ClientAdmissionIntent::Resume {
                    player_id: resume.player_id,
                    ship_id: resume.ship_id,
                },
                None => ClientAdmissionIntent::Fresh { spawn_position },
            };

            let mut attempt = match node.begin_client_admission(intent, AOI_CELL_SIZE) {
                Ok(attempt) => attempt,
                Err(refusal) => {
                    log_single_refusal(request.peer_addr, refusal);
                    continue;
                }
            };
            let player_id = attempt.player_id();
            let ship_id = attempt.ship_id();
            let payload = attempt.take_handoff_payload();
            let tx = completion_tx.clone();

            tokio::spawn(async move {
                let result = request
                    .complete(
                        player_id,
                        ship_id,
                        payload.initial_state,
                        payload.player_loadout,
                    )
                    .await
                    .map_err(|error| error.to_string());
                let _ = tx.send((attempt, result));
            });
        }

        let events_before: u64 = node.total_event_count() as u64;

        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();
        for sess in sessions.iter_mut() {
            while let Some(market_command) = sess.try_recv_market_command() {
                let snapshot = market.handle_single(sess.player_id, market_command, &mut node);
                sess.send_message(&ServerMessage::MarketSnapshot(snapshot));
            }
            while let Some(cmd) = sess.try_recv_command() {
                match node.apply_client_command(sess.player_id, cmd, &mut lock_commands) {
                    Some(ClientCommandFollowup::Jump { ship_id, command }) => {
                        eprintln!(
                            "[Server] JumpCommand ignored (ship #{} gate #{}): \
                             --serve runs a single-sector node without Raft",
                            ship_id.raw(),
                            command.gate_id.0
                        );
                    }
                    Some(followup @ ClientCommandFollowup::RefreshPlayerLoadout { .. }) => {
                        if let Some(player_id) = followup.loadout_player_id() {
                            if let Some(loadout) =
                                node.build_player_loadout_json_for_player(player_id)
                            {
                                sess.send_message(&ServerMessage::PlayerLoadout(loadout));
                            }
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
                if let Some(loadout) = node.build_player_loadout_json(sess.ship_id) {
                    sess.send_message(&ServerMessage::PlayerLoadout(loadout));
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

fn log_single_refusal(addr: std::net::SocketAddr, refusal: ClientAdmissionRefusal) {
    match refusal {
        ClientAdmissionRefusal::FreshAtPopulationCap => {
            eprintln!("[Server] connection from {addr} refused: Sector at population cap");
        }
        ClientAdmissionRefusal::ResumeShipMissing { ship_id, .. } => {
            eprintln!(
                "[Server] resume from {addr} refused: ship #{} is not in this Sector",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {
            eprintln!("[Server] handshake from {addr} refused: {error}");
        }
    }
}

fn finish_single_admission<S: EventStore, T>(
    node: &mut SimulationNode<S>,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match result {
        Ok(value) => match attempt.commit(node) {
            Ok(committed) => Some((value, committed)),
            Err(error) => {
                eprintln!("[Server] {error}");
                None
            }
        },
        Err(error) => {
            attempt.abort(node);
            eprintln!("[Server] handshake failed: {error}");
            None
        }
    }
}

fn send_post_commit_loadout<S: EventStore>(
    node: &SimulationNode<S>,
    session: &ws_server::PlayerSession,
    committed: CommittedClientAdmission,
) {
    if !committed.resumed {
        return;
    }
    if let Some(loadout) = node.build_player_loadout_json(committed.ship_id) {
        session.send_message(&ServerMessage::PlayerLoadout(loadout));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{PlayerId, ShipTypeId, Velocity};

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn single_adapter_commits_successful_attempt() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");

        assert_eq!(
            finish_single_admission(&mut node, attempt, Ok::<_, String>(()))
                .map(|(value, _)| value),
            Some(())
        );
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn single_adapter_disconnect_aborts_fresh_attempt() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");

        assert_eq!(
            finish_single_admission::<_, ()>(
                &mut node,
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn single_adapter_disconnect_does_not_remove_resumed_ship() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");

        assert_eq!(
            finish_single_admission::<_, ()>(
                &mut node,
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 1);
        assert!(node.ship_absolute_pos(ship_id).is_some());
    }
}
