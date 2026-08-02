//! Raft-cluster WebSocket server (`--serve --cluster`, ADR-0009/0014).

use super::{
    build_serve_node, market::MarketRuntime, runtime, AoiDelivery, AOI_CELL_SIZE, P4_TICK_MS,
};
use crate::{cluster, ws_server};
use dawn_core::{DomainEvent, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal,
    CommittedClientAdmission,
};
use dawn_sector::node::{ClientCommandFollowup, JumpOutcome, SimulationNode};
use dawn_sector::transit;
use dawn_wire::ServerMessage;
use std::collections::HashMap;
use tokio::sync::mpsc;

type HandshakeCompletion = (
    ClientAdmissionAttempt,
    Result<ws_server::PlayerSession, String>,
);

pub(crate) async fn run_cluster_server(ship_count: usize, pop_cap: usize) {
    const SECTORS: usize = 3;
    /// 2x the Alpha star (Helios) radius from Sector origin (matches
    /// SimulationNode::DEFAULT_PLAYER_SPAWN): clear of the star body itself,
    /// far short of Gate 0 (600,000 units, at the Sector edge), and well beyond
    /// the 3,000u warp minimum, so warp/approach to the gate both work (ADR-0022).
    const PLAYER_SPAWN: Position = Position {
        x: 30_000.0,
        y: 0.0,
        z: 0.0,
    };

    println!("═══════════════════════════════════════════");
    println!("  Phase 7.5 — Raft cluster WebSocket server ");
    println!("═══════════════════════════════════════════");
    println!("  sectors  : {SECTORS} (one Raft node each)");
    println!("  npc ships: {ship_count} in Sector 0  (change with --ships N)");
    println!(
        "  tick rate: {} ms/tick  ({} tick/sec)",
        P4_TICK_MS,
        1000 / P4_TICK_MS
    );
    println!("  travel   : select Gate 0 (click its ring), press W to warp (or A to approach),");
    println!("             then J to jump once in range (player spawns at the Sector origin)");
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = ws_server::WsServer::bind("127.0.0.1:7878")
        .await
        .expect("failed to bind WebSocket server");

    let ids: Vec<NodeId> = (0..SECTORS as u8).map(NodeId).collect();
    let (endpoints, _partitioned) = cluster::spawn_raft_actors(&ids);
    let (rafts, mut committed_rxs): (Vec<_>, Vec<_>) = endpoints.into_iter().unzip();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut nodes: Vec<SimulationNode> = ids
        .iter()
        .map(|&id| build_serve_node(id, SectorId(id.0), bounds, pop_cap))
        .collect();
    let mut market = MarketRuntime::open("data/market.sqlite")
        .expect("failed to open Market database at data/market.sqlite");

    nodes[0].spawn_npc_frigates(ship_count);

    // Warm up: tick until a Raft leader is elected (election timeout ≤ 20 ticks).
    for _ in 0..30 {
        for i in 0..SECTORS {
            let _ = transit::run_runtime_tick(
                &mut nodes[i],
                &rafts[i],
                &mut committed_rxs[i],
                &[],
                |_, _, _| {},
            );
        }
    }
    println!("  [Server] Raft warm-up complete. Waiting for players...");

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
    let mut player_sector: HashMap<PlayerId, usize> = HashMap::new();
    let mut ship_player: HashMap<ShipId, PlayerId> = HashMap::new();
    let mut aoi_delivery = AoiDelivery::new(AOI_CELL_SIZE);

    let mut interval = tokio::time::interval(std::time::Duration::from_millis(P4_TICK_MS));

    loop {
        interval.tick().await;

        // Commit Sector ownership before publishing cluster routing. A failed
        // or disconnected handshake therefore leaves neither route map visible.
        while let Ok((attempt, result)) = completion_rx.try_recv() {
            if let Some((sess, committed)) = finish_cluster_admission(
                &mut nodes[0],
                &mut player_sector,
                &mut ship_player,
                attempt,
                result,
            ) {
                send_post_commit_loadout(&nodes[0], &sess, committed);
                println!(
                    "  [Server] {} joined with ship #{}",
                    sess.player_id,
                    sess.ship_id.raw()
                );
                aoi_delivery.seed_cluster_player(&nodes, 0, sess.player_id, sess.ship_id);
                sessions.push(sess);
            }
        }

        while let Ok(request) = handshake_req_rx.try_recv() {
            let intent = match request.resume {
                Some(resume) => ClientAdmissionIntent::Resume {
                    player_id: resume.player_id,
                    ship_id: resume.ship_id,
                },
                None => ClientAdmissionIntent::Fresh {
                    spawn_position: PLAYER_SPAWN,
                },
            };
            let mut attempt = match nodes[0].begin_client_admission(intent, AOI_CELL_SIZE) {
                Ok(attempt) => attempt,
                Err(refusal) => {
                    log_cluster_refusal(request.peer_addr, refusal);
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

        let mut lock_commands: Vec<Vec<dawn_core::LockOnCommand>> = vec![Vec::new(); SECTORS];

        for sess in sessions.iter_mut() {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            while let Some(market_command) = sess.try_recv_market_command() {
                let snapshot =
                    market.handle_cluster(sess.player_id, market_command, sector, &mut nodes);
                sess.send_message(&ServerMessage::MarketSnapshot(snapshot));
            }
            while let Some(cmd) = sess.try_recv_command() {
                let followup = nodes[sector].apply_client_command(
                    sess.player_id,
                    cmd,
                    &mut lock_commands[sector],
                );
                let (ship_id, j) = match followup {
                    Some(ClientCommandFollowup::Jump { ship_id, command }) => (ship_id, command),
                    Some(followup @ ClientCommandFollowup::RefreshPlayerLoadout { .. }) => {
                        if let Some(player_id) = followup.loadout_player_id() {
                            if let Some(loadout) =
                                nodes[sector].build_player_loadout_json_for_player(player_id)
                            {
                                sess.send_message(&ServerMessage::PlayerLoadout(loadout));
                            }
                        }
                        continue;
                    }
                    None => continue,
                };
                if ship_id != sess.ship_id {
                    continue;
                }
                match transit::propose_jump(&mut nodes[sector], &rafts[sector], ship_id, j.gate_id)
                {
                    JumpOutcome::NeedsTransitProposal { to } => {
                        println!(
                            "  [Server] Jump proposed: ship #{} gate #{} (S{} → S{})",
                            ship_id.raw(),
                            j.gate_id.0,
                            sector,
                            to.0
                        );
                    }
                    JumpOutcome::WarpFallbackStarted => {
                        println!(
                            "  [Server] Jump: ship #{} out of range — auto-warp to gate #{} started",
                            ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                    JumpOutcome::ApproachFallbackStarted => {
                        // Too close to warp (< MIN_WARP_DISTANCE) but still outside
                        // activation_radius -- without this, a ship in that band
                        // could never jump: in_range fails, and apply_warp_command
                        // also fails its own can_propose_warp distance check, so
                        // the command was silently dropped every tick the ship sat
                        // there. Approach closes the rest of the gap sublight.
                        println!(
                            "  [Server] Jump: ship #{} too close to warp — approaching gate #{} instead",
                            ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                    JumpOutcome::Rejected => {
                        eprintln!(
                            "[Server] JumpCommand rejected (ship #{} gate #{})",
                            ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                }
            }
        }

        let tick_results = runtime::run_cluster_runtime_tick(
            runtime::ClusterRuntimeTickContext {
                nodes: &mut nodes,
                rafts: &rafts,
                committed_rxs: &mut committed_rxs,
                sessions: &mut sessions,
                player_sector: &mut player_sector,
                ship_player: &ship_player,
                aoi_delivery: &mut aoi_delivery,
            },
            &lock_commands,
        );

        for sess in &sessions {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            let should_refresh = tick_results[sector].events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::ShipDestroyed(destroyed)
                        if destroyed.killer_id == sess.ship_id
                )
            });
            if should_refresh {
                if let Some(loadout) = nodes[sector].build_player_loadout_json(sess.ship_id) {
                    sess.send_message(&ServerMessage::PlayerLoadout(loadout));
                }
            }
        }
    }
}

fn log_cluster_refusal(addr: std::net::SocketAddr, refusal: ClientAdmissionRefusal) {
    match refusal {
        ClientAdmissionRefusal::FreshAtPopulationCap => {
            eprintln!("[Server] connection from {addr} refused: Sector 0 at population cap");
        }
        ClientAdmissionRefusal::ResumeShipMissing { ship_id, .. } => {
            eprintln!(
                "[Server] clustered resume from {addr} refused: ship #{} is not in Sector 0",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {
            eprintln!("[Server] clustered handshake from {addr} refused: {error}");
        }
    }
}

fn finish_cluster_admission<S: EventStore, T>(
    node: &mut SimulationNode<S>,
    player_sector: &mut HashMap<PlayerId, usize>,
    ship_player: &mut HashMap<ShipId, PlayerId>,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match result {
        Ok(value) => match attempt.commit(node) {
            Ok(committed) => {
                player_sector.insert(committed.player_id, 0);
                ship_player.insert(committed.ship_id, committed.player_id);
                Some((value, committed))
            }
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
    use dawn_core::{ShipTypeId, Velocity};

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn cluster_adapter_publishes_routes_only_after_commit() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let player_id = attempt.player_id();
        let ship_id = attempt.ship_id();
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();
        assert!(player_sector.is_empty() && ship_player.is_empty());

        assert_eq!(
            finish_cluster_admission(
                &mut node,
                &mut player_sector,
                &mut ship_player,
                attempt,
                Ok::<_, String>(()),
            )
            .map(|(value, _)| value),
            Some(())
        );

        assert_eq!(player_sector.get(&player_id), Some(&0));
        assert_eq!(ship_player.get(&ship_id), Some(&player_id));
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn cluster_adapter_disconnect_rolls_back_fresh_spawn_and_routes() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();

        assert_eq!(
            finish_cluster_admission::<_, ()>(
                &mut node,
                &mut player_sector,
                &mut ship_player,
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );

        assert_eq!(node.ship_count(), 0);
        assert!(player_sector.is_empty());
        assert!(ship_player.is_empty());
    }

    #[test]
    fn cluster_adapter_failed_resume_keeps_ship_and_routes_absent() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();

        assert_eq!(
            finish_cluster_admission::<_, ()>(
                &mut node,
                &mut player_sector,
                &mut ship_player,
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );

        assert_eq!(node.ship_count(), 1);
        assert!(node.ship_absolute_pos(ship_id).is_some());
        assert!(player_sector.is_empty());
        assert!(ship_player.is_empty());
    }
}
