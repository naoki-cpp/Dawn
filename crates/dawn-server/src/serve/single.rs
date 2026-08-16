//! Single-node WebSocket server (`--serve`, no Raft cluster).
#![allow(clippy::module_name_repetitions)]

use super::{
    build_serve_node, client_request_rejection, load_serve_dependencies, market::MarketRuntime,
    AoiDelivery, DuelMetrics, AOI_CELL_SIZE, P4_TICK_MS, TIDI_BUDGET,
};
use crate::runtime_frame::{RuntimeFrameHost, RuntimeFramePolicy};
use crate::ws_server;
use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, ShipId};
use dawn_protocol::ServerMessage;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal, CommittedClientAdmission,
};
use dawn_sector::client_admission_resolution::{
    resolve_client_admission, ClientAdmissionResolution,
};
use dawn_sector::dilation;
use dawn_sector::node::{RuntimeCommandDispatch, SimulationNode};
use dawn_sector::transit::LocalRuntimeConsensus;
use dawn_storage::InMemoryJournal;
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
    let (galaxy, catalog) = load_serve_dependencies();
    let node = build_serve_node(NodeId(0), SectorId(0), bounds, pop_cap, galaxy, catalog);
    let mut host = RuntimeFrameHost::new(
        node,
        InMemoryJournal::new(),
        LocalRuntimeConsensus,
        RuntimeFramePolicy::local_durable(0),
    );
    let mut market = MarketRuntime::open("data/market.sqlite")
        .expect("failed to open Market database at data/market.sqlite");

    host.spawn_npc_frigates(ship_count);
    // Duel-mode player spawn: close enough to the Bot to be within weapon
    // range (Small Railgun: 3000 range + 2000 falloff = 5000) from the
    // moment the human connects, instead of the universe-wide
    // station-derived spawn (so far from the Bot's
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
            let (_, bot_ship_id) = host.spawn_bot_ship(bot_pos);
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
    // Settlement identities the Market ledger decided last tick, retired
    // from the Sector's idempotency guard by the next frame (issue #315).
    let mut decided_settlements: Vec<u64> = Vec::new();

    loop {
        interval.tick().await;

        // Resolve socket outcomes on the tick-loop thread. A session is not
        // visible to AoI delivery or command routing until its Sector attempt
        // commits successfully.
        for (sess, _committed) in host.drain_single_admission_completions(&mut completion_rx) {
            println!(
                "  [Server] {} joined with ship #{}",
                sess.player_id,
                sess.ship_id.raw()
            );
            sessions.retain(|existing| {
                existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
            });
            aoi_delivery.seed_single_player(host.node(), sess.player_id, sess.ship_id);

            if duel_mode && player_ship_id.is_none() {
                player_ship_id = Some(sess.ship_id);
                let tick = host.node().current_tick().value();
                duel_metrics = Some(DuelMetrics::new(tick));
                println!("  [Duel] metrics collection started at tick {tick}");
            }

            sessions.push(sess);
        }

        while let Ok(request) = handshake_req_rx.try_recv() {
            let spawn_position = if duel_mode {
                duel_player_spawn
            } else {
                host.node().default_player_spawn_position()
            };
            let intent = match request.resume {
                Some(resume) => ClientAdmissionIntent::Resume {
                    resume_ticket: resume,
                },
                None => ClientAdmissionIntent::Fresh { spawn_position },
            };

            let mut attempt = match host.begin_client_admission(intent, AOI_CELL_SIZE) {
                Ok(attempt) => attempt,
                Err(refusal) => {
                    log_single_refusal(request.peer_addr, refusal);
                    continue;
                }
            };
            let player_id = attempt.player_id();
            let ship_id = attempt.ship_id();
            let resume_ticket = attempt.resume_ticket();
            let payload = attempt.take_handoff_payload();
            let tx = completion_tx.clone();

            tokio::spawn(async move {
                let result = request
                    .complete(
                        player_id,
                        ship_id,
                        resume_ticket,
                        payload.initial_state,
                        payload.player_loadout,
                    )
                    .await
                    .map_err(|error| error.to_string());
                let _ = tx.send((attempt, result));
            });
        }

        // Events from the previous transition have already been delivered (or
        // intentionally ignored by this single-sector adapter). Start this
        // transition with an empty output buffer rather than deriving output
        // from the legacy event-log cursor.
        let _ = host.drain_pending_events();
        let mut all_new_events = Vec::new();

        let mut lock_commands: Vec<dawn_core::LockOnCommand> = Vec::new();
        for sess in sessions.iter_mut() {
            while let Some(market_command) = sess.try_recv_market_command() {
                let snapshot = market.handle_single(sess.player_id, market_command, host.node());
                sess.send_message(&ServerMessage::MarketSnapshot(snapshot));
            }
        }
        for dispatch in host.collect_runtime_commands(
            &mut sessions,
            &mut lock_commands,
            |session| session.player_id,
            ws_server::PlayerSession::try_recv_request,
        ) {
            match dispatch {
                RuntimeCommandDispatch::Jump {
                    ship_id, command, ..
                } => {
                    eprintln!(
                        "[Server] JumpCommand ignored (ship #{} gate #{}): \
                         --serve runs a single-sector node without Raft",
                        ship_id.raw(),
                        command.gate_id.0
                    );
                }
                RuntimeCommandDispatch::RefreshPlayerLoadout {
                    session_index,
                    player_id,
                } => {
                    if let Some(loadout) =
                        host.node().build_player_loadout_json_for_player(player_id)
                    {
                        if let Some(session) = sessions.get_mut(session_index) {
                            session.send_message(&ServerMessage::PlayerLoadout(loadout));
                        }
                    }
                }
                RuntimeCommandDispatch::Rejected {
                    session_index,
                    error,
                } => {
                    if let Some(session) = sessions.get_mut(session_index) {
                        session.send_message(&ServerMessage::ClientRequestRejected(
                            client_request_rejection(error),
                        ));
                    }
                }
            }
        }

        // Drain still-pending Market settlements against the live node once
        // per tick, right before the frame that will carry them (issue
        // #315): this keeps a settlement's cargo mutation inside the same
        // durable write set as everything else in the tick instead of
        // mutating live state synchronously outside any tick boundary.
        let queued_settlements = market.drain_settlements(host.node());
        let market_settlements: Vec<dawn_sector::transition::MarketSettlementInput> =
            queued_settlements
                .iter()
                .map(|queued| queued.input())
                .collect();

        let output = host
            .run_frame(dawn_sector::transition::FrameInput {
                lock_commands: &lock_commands,
                market_settlements: &market_settlements,
                acknowledged_settlements: &decided_settlements,
            })
            .expect("in-memory single-sector runtime must accept durable Tick");
        let tick_result = output.tick_result;
        let acknowledgement = market
            .acknowledge_settlements(&queued_settlements, &tick_result.market_settlement_outcomes);
        // Retired on the next frame, through the same durable boundary as
        // everything else (issue #315).
        decided_settlements = acknowledgement.decided_settlement_ids;
        // The client was told "settlement pending" when it placed the order;
        // now that the outcome is known, say so instead of leaving it there.
        for player_id in acknowledgement.rejected_players {
            let snapshot = market.settlement_rejected_snapshot(player_id);
            if let Some(session) = sessions.iter_mut().find(|s| s.player_id == player_id) {
                session.send_message(&ServerMessage::MarketSnapshot(snapshot));
            }
        }
        all_new_events.extend(output.events);

        for sess in &sessions {
            let should_refresh = tick_result.events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::ShipDestroyed(destroyed)
                        if destroyed.killer_id == sess.ship_id
                )
            });
            if should_refresh {
                if let Some(loadout) = host.node().build_player_loadout_json(sess.ship_id) {
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

        let warp_arrivals = output.completed_warps;
        aoi_delivery.deliver_single_sector(
            host.node(),
            &mut sessions,
            &all_new_events,
            &warp_arrivals,
        );

        let was_dilated = tidi.is_dilated();
        let prior_active = tidi.active_ticks();
        tidi.update(host.node().ship_count() as f64);
        if tidi.is_dilated() {
            if !was_dilated {
                println!(
                    "[TiDi] dilation engaged: factor={:.2} (cost {} > budget {TIDI_BUDGET:.0})",
                    tidi.dilation(),
                    host.node().ship_count()
                );
            }
            let extra = tidi.paced_tick_ms(P4_TICK_MS as f64) - P4_TICK_MS as f64;
            tokio::time::sleep(std::time::Duration::from_millis(extra as u64)).await;
        } else if was_dilated {
            println!("[TiDi] dilation recovered to real-time after {prior_active} ticks (cost {} <= budget {TIDI_BUDGET:.0})",
                host.node().ship_count());
        }
    }
}

fn log_single_refusal(addr: std::net::SocketAddr, refusal: ClientAdmissionRefusal) {
    match refusal {
        ClientAdmissionRefusal::FreshAtPopulationCap => {
            eprintln!("[Server] connection from {addr} refused: Sector at population cap");
        }
        ClientAdmissionRefusal::ResumeTicketInvalid => {
            eprintln!("[Server] resume from {addr} refused: invalid resume ticket");
        }
        ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. } => {
            eprintln!(
                "[Server] resume from {addr} refused: ship #{} already has an in-flight resume",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. } => {
            eprintln!(
                "[Server] resume from {addr} refused: ship #{} conflicts with established ownership",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {
            eprintln!("[Server] handshake from {addr} refused: {error}");
        }
    }
}

fn drain_single_admission_completions(
    node: &mut SimulationNode,
    completion_rx: &mut mpsc::UnboundedReceiver<HandshakeCompletion>,
) -> Vec<(ws_server::PlayerSession, CommittedClientAdmission)> {
    let mut ready = Vec::new();
    while let Ok((attempt, result)) = completion_rx.try_recv() {
        if let Some(completed) = finish_single_admission(node, attempt, result) {
            ready.push(completed);
        }
    }
    ready
}

impl RuntimeFrameHost<InMemoryJournal, LocalRuntimeConsensus> {
    /// Resolve async handshake outcomes against the owned node, promoting
    /// each committed attempt to a ready session.
    fn drain_single_admission_completions(
        &mut self,
        completion_rx: &mut mpsc::UnboundedReceiver<HandshakeCompletion>,
    ) -> Vec<(ws_server::PlayerSession, CommittedClientAdmission)> {
        self.with_node_mut(|node| drain_single_admission_completions(node, completion_rx))
    }
}

fn finish_single_admission<T>(
    node: &mut SimulationNode,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match resolve_client_admission(node, attempt, result) {
        ClientAdmissionResolution::Committed { value, admission } => Some((value, admission)),
        ClientAdmissionResolution::Aborted { error } => {
            eprintln!("[Server] handshake failed: {error}");
            None
        }
        ClientAdmissionResolution::CommitRejected { error } => {
            eprintln!("[Server] {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
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
            finish_single_admission::<()>(
                &mut node,
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn single_adapter_drains_async_disconnect_completion() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let (completion_tx, mut completion_rx) = mpsc::unbounded_channel::<HandshakeCompletion>();
        completion_tx
            .send((attempt, Err("client disconnected".to_string())))
            .expect("completion receiver alive");

        let ready = drain_single_admission_completions(&mut node, &mut completion_rx);

        assert!(ready.is_empty());
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn single_adapter_disconnect_does_not_remove_resumed_ship() {
        let mut node = node();
        let fresh = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let resume_ticket = fresh.resume_ticket();
        let committed = match resolve_client_admission(&mut node, fresh, Ok::<_, ()>(())) {
            ClientAdmissionResolution::Committed { admission, .. } => admission,
            other => panic!("fresh admission should commit, got {other:?}"),
        };
        let ship_id = committed.ship_id;
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");

        assert_eq!(
            finish_single_admission::<()>(
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
