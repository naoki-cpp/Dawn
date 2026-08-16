//! Raft-cluster WebSocket server (`--serve --cluster`, ADR-0009/0014).
#![allow(clippy::module_name_repetitions)]

use super::{
    build_serve_node, client_request_rejection, load_serve_dependencies, market::MarketRuntime,
    runtime, AoiDelivery, AOI_CELL_SIZE, P4_TICK_MS,
};
use crate::runtime_frame::{
    OwnedRaftRuntimeConsensus, RuntimeFrameHost, RuntimeFramePolicy, RuntimeNodeMutation,
    RuntimeNodeView,
};
use crate::{cluster, ws_server};
use dawn_core::{DomainEvent, NodeId, PlayerId, SectorBounds, SectorId, ShipId};
use dawn_protocol::ServerMessage;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal, CommittedClientAdmission,
};
use dawn_sector::client_admission_resolution::{
    resolve_client_admission, ClientAdmissionResolution,
};
use dawn_sector::node::{JumpOutcome, RuntimeCommandDispatch, SimulationNode};
use dawn_storage::InMemoryJournal;
use std::collections::HashMap;
use tokio::sync::mpsc;

type HandshakeCompletion = (
    usize,
    ClientAdmissionAttempt,
    Result<ws_server::PlayerSession, String>,
);

pub(crate) async fn run_cluster_server(ship_count: usize, pop_cap: usize) {
    const SECTORS: usize = 3;
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
    println!(
        "             then J to jump once in range (fresh players begin at the local station)"
    );
    println!();
    println!("  Open Godot client and press Play (F5)");
    println!("  Press Ctrl-C to stop");
    println!();

    let server = ws_server::WsServer::bind("127.0.0.1:7878")
        .await
        .expect("failed to bind WebSocket server");

    let ids: Vec<NodeId> = (0..SECTORS as u8).map(NodeId).collect();
    let (endpoints, _partitioned) = cluster::spawn_raft_actors(&ids);
    let (rafts, committed_rxs): (Vec<_>, Vec<_>) = endpoints.into_iter().unzip();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let (galaxy, catalog) = load_serve_dependencies();
    let mut hosts: Vec<RuntimeFrameHost<InMemoryJournal, OwnedRaftRuntimeConsensus>> = ids
        .iter()
        .zip(rafts.into_iter().zip(committed_rxs))
        .map(|(&id, (raft, committed_rx))| {
            RuntimeFrameHost::new(
                build_serve_node(
                    id,
                    SectorId(id.0),
                    bounds,
                    pop_cap,
                    std::sync::Arc::clone(&galaxy),
                    std::sync::Arc::clone(&catalog),
                ),
                InMemoryJournal::new(),
                OwnedRaftRuntimeConsensus::new(raft, committed_rx),
                RuntimeFramePolicy::local_durable(0),
            )
        })
        .collect();
    let mut market = MarketRuntime::open("data/market.sqlite")
        .expect("failed to open Market database at data/market.sqlite");

    hosts[0].with_node_mut(|node| node.spawn_npc_frigates(ship_count));

    // Warm up: tick until a Raft leader is elected (election timeout ≤ 20 ticks).
    for _ in 0..30 {
        for host in &mut hosts {
            let _ = host.run_frame(dawn_sector::transition::FrameInput::lock_only(&[]));
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
        for (sector, sess, _committed) in drain_cluster_admission_completions(
            &mut hosts,
            &mut player_sector,
            &mut ship_player,
            &mut completion_rx,
        ) {
            println!(
                "  [Server] {} joined with ship #{} in Sector {}",
                sess.player_id,
                sess.ship_id.raw(),
                sector
            );
            sessions.retain(|existing| {
                existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
            });
            aoi_delivery.seed_cluster_player(&hosts, sector, sess.player_id, sess.ship_id);
            sessions.push(sess);
        }
        while let Ok(request) = handshake_req_rx.try_recv() {
            let (sector, intent) = match request.resume {
                Some(resume) => {
                    let Some(sector) = find_resume_sector(&hosts, resume) else {
                        log_cluster_refusal(
                            request.peer_addr,
                            ClientAdmissionRefusal::ResumeTicketInvalid,
                        );
                        continue;
                    };
                    (
                        sector,
                        ClientAdmissionIntent::Resume {
                            resume_ticket: resume,
                        },
                    )
                }
                None => {
                    let sector = 0;
                    let spawn_position =
                        hosts[sector].with_node_mut(|node| node.default_player_spawn_position());
                    (sector, ClientAdmissionIntent::Fresh { spawn_position })
                }
            };
            let mut attempt = match hosts[sector]
                .with_node_mut(|node| node.begin_client_admission(intent, AOI_CELL_SIZE))
            {
                Ok(attempt) => attempt,
                Err(refusal) => {
                    log_cluster_refusal(request.peer_addr, refusal);
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
                let _ = tx.send((sector, attempt, result));
            });
        }

        let mut lock_commands: Vec<Vec<dawn_core::LockOnCommand>> = vec![Vec::new(); SECTORS];

        for sess in sessions.iter_mut() {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            while let Some(market_command) = sess.try_recv_market_command() {
                let snapshot =
                    market.handle_cluster(sess.player_id, market_command, sector, &mut hosts);
                sess.send_message(&ServerMessage::MarketSnapshot(snapshot));
            }
            let dispatches = hosts[sector].collect_runtime_commands(
                std::slice::from_mut(sess),
                &mut lock_commands[sector],
                |session| session.player_id,
                ws_server::PlayerSession::try_recv_request,
            );
            for dispatch in dispatches {
                match dispatch {
                    RuntimeCommandDispatch::Jump {
                        ship_id, command, ..
                    } => {
                        if ship_id != sess.ship_id {
                            continue;
                        }
                        match hosts[sector].propose_jump(ship_id, command.gate_id) {
                            JumpOutcome::NeedsTransitProposal { to } => {
                                println!(
                                    "  [Server] Jump proposed: ship #{} gate #{} (S{} → S{})",
                                    ship_id.raw(),
                                    command.gate_id.0,
                                    sector,
                                    to.0
                                );
                            }
                            JumpOutcome::WarpFallbackStarted => {
                                println!(
                            "  [Server] Jump: ship #{} out of range — auto-warp to gate #{} started",
                            ship_id.raw(),
                            command.gate_id.0
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
                            command.gate_id.0
                        );
                            }
                            JumpOutcome::Rejected => {
                                eprintln!(
                                    "[Server] JumpCommand rejected (ship #{} gate #{})",
                                    ship_id.raw(),
                                    command.gate_id.0
                                );
                            }
                        }
                    }
                    RuntimeCommandDispatch::RefreshPlayerLoadout { player_id, .. } => {
                        if let Some(loadout) = hosts[sector]
                            .node()
                            .build_player_loadout_json_for_player(player_id)
                        {
                            sess.send_message(&ServerMessage::PlayerLoadout(loadout));
                        }
                    }
                    RuntimeCommandDispatch::Rejected { error, .. } => {
                        sess.send_message(&ServerMessage::ClientRequestRejected(
                            client_request_rejection(error),
                        ));
                    }
                }
            }
        }

        let tick_results = runtime::run_cluster_runtime_tick(
            runtime::ClusterRuntimeTickContext {
                hosts: &mut hosts,
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
                if let Some(loadout) = hosts[sector].node().build_player_loadout_json(sess.ship_id)
                {
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
        ClientAdmissionRefusal::ResumeTicketInvalid => {
            eprintln!("[Server] clustered resume from {addr} refused: invalid resume ticket");
        }
        ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. } => {
            eprintln!(
                "[Server] clustered resume from {addr} refused: ship #{} already has an in-flight resume",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. } => {
            eprintln!(
                "[Server] clustered resume from {addr} refused: ship #{} conflicts with established ownership",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {
            eprintln!("[Server] clustered handshake from {addr} refused: {error}");
        }
    }
}

fn find_resume_sector<N: RuntimeNodeView>(
    nodes: &[N],
    resume_ticket: dawn_core::ResumeTicket,
) -> Option<usize> {
    let mut sectors = nodes.iter().enumerate().filter_map(|(sector, node)| {
        node.runtime_node()
            .hosts_client_resume_ticket(resume_ticket)
            .then_some(sector)
    });
    let sector = sectors.next()?;
    if sectors.next().is_some() {
        None
    } else {
        Some(sector)
    }
}

fn drain_cluster_admission_completions<N: RuntimeNodeMutation>(
    nodes: &mut [N],
    player_sector: &mut HashMap<PlayerId, usize>,
    ship_player: &mut HashMap<ShipId, PlayerId>,
    completion_rx: &mut mpsc::UnboundedReceiver<HandshakeCompletion>,
) -> Vec<(usize, ws_server::PlayerSession, CommittedClientAdmission)> {
    let mut ready = Vec::new();
    while let Ok((sector, attempt, result)) = completion_rx.try_recv() {
        if let Some((session, committed)) = nodes[sector].with_runtime_node_mut(|node| {
            finish_cluster_admission(node, sector, player_sector, ship_player, attempt, result)
        }) {
            ready.push((sector, session, committed));
        }
    }
    ready
}

fn finish_cluster_admission<T>(
    node: &mut SimulationNode,
    sector: usize,
    player_sector: &mut HashMap<PlayerId, usize>,
    ship_player: &mut HashMap<ShipId, PlayerId>,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match resolve_client_admission(node, attempt, result) {
        ClientAdmissionResolution::Committed { value, admission } => {
            player_sector.retain(|player_id, _| *player_id != admission.player_id);
            ship_player.retain(|ship_id, player_id| {
                *ship_id != admission.ship_id && *player_id != admission.player_id
            });
            player_sector.insert(admission.player_id, sector);
            ship_player.insert(admission.ship_id, admission.player_id);
            Some((value, admission))
        }
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
    use dawn_core::Position;

    fn node_at(sector: u8) -> SimulationNode {
        SimulationNode::new(
            NodeId(sector),
            SectorId(sector),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        )
    }

    fn node() -> SimulationNode {
        node_at(0)
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
                0,
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
            finish_cluster_admission::<()>(
                &mut node,
                0,
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
    fn cluster_adapter_drains_async_disconnect_completion() {
        let mut nodes = vec![node()];
        let attempt = nodes[0]
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let (completion_tx, mut completion_rx) = mpsc::unbounded_channel::<HandshakeCompletion>();
        completion_tx
            .send((0, attempt, Err("client disconnected".to_string())))
            .expect("completion receiver alive");
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();

        let ready = drain_cluster_admission_completions(
            &mut nodes,
            &mut player_sector,
            &mut ship_player,
            &mut completion_rx,
        );

        assert!(ready.is_empty());
        assert_eq!(nodes[0].ship_count(), 0);
        assert!(player_sector.is_empty());
        assert!(ship_player.is_empty());
    }

    #[test]
    fn cluster_adapter_resumes_ship_in_its_current_sector() {
        let mut nodes = vec![node_at(0), node_at(1)];
        let fresh = nodes[1]
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let resume_ticket = fresh.resume_ticket();
        let committed = match resolve_client_admission(&mut nodes[1], fresh, Ok::<_, ()>(())) {
            ClientAdmissionResolution::Committed { admission, .. } => admission,
            other => panic!("fresh admission should commit, got {other:?}"),
        };
        let player_id = committed.player_id;
        let ship_id = committed.ship_id;
        let sector = find_resume_sector(&nodes, resume_ticket).expect("unique owning Sector");
        let attempt = nodes[sector]
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();

        assert_eq!(
            finish_cluster_admission(
                &mut nodes[sector],
                sector,
                &mut player_sector,
                &mut ship_player,
                attempt,
                Ok::<_, String>(()),
            )
            .map(|(value, _)| value),
            Some(())
        );

        assert_eq!(sector, 1);
        assert_eq!(player_sector.get(&player_id), Some(&1));
        assert_eq!(ship_player.get(&ship_id), Some(&player_id));
    }

    #[test]
    fn cluster_adapter_routes_prepared_fresh_identity_to_its_sector() {
        let mut nodes = vec![node_at(0), node_at(1)];
        let attempt = nodes[1]
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::new(30_000.0, 0.0, 0.0),
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let ship_id = attempt.ship_id();
        let resume_ticket = attempt.resume_ticket();
        assert!(matches!(
            resolve_client_admission::<(), _>(&mut nodes[1], attempt, Err(())),
            ClientAdmissionResolution::Aborted { .. }
        ));
        assert!(nodes[1].ship_absolute_pos(ship_id).is_none());

        let sector = find_resume_sector(&nodes, resume_ticket).expect("prepared identity Sector");
        assert_eq!(sector, 1);
        let recovered = nodes[sector]
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("exact prepared identity resumes in its Sector");
        assert!(!recovered.is_resumed());
        assert!(matches!(
            resolve_client_admission::<(), _>(&mut nodes[sector], recovered, Err(())),
            ClientAdmissionResolution::Aborted { .. }
        ));
    }

    #[test]
    fn cluster_adapter_failed_resume_keeps_ship_and_routes_absent() {
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
        let mut player_sector = HashMap::new();
        let mut ship_player = HashMap::new();

        assert_eq!(
            finish_cluster_admission::<()>(
                &mut node,
                0,
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
