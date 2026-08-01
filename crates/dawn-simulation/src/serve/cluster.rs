//! Raft-cluster WebSocket server (`--serve --cluster`, ADR-0009/0014).

use super::{
    build_serve_node, market::MarketRuntime, runtime, AoiDelivery, AOI_CELL_SIZE, P4_TICK_MS,
};
use crate::{cluster, ws_server};
use dawn_core::{DomainEvent, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId};
use dawn_sector::node::{ClientCommandFollowup, JumpOutcome, SimulationNode};
use dawn_sector::transit;
use dawn_wire::ServerMessage;
use std::collections::HashMap;
use tokio::sync::mpsc;

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
    let mut player_sector: HashMap<PlayerId, usize> = HashMap::new();
    let mut ship_player: HashMap<ShipId, PlayerId> = HashMap::new();
    let mut aoi_delivery = AoiDelivery::new(AOI_CELL_SIZE);

    let mut interval = tokio::time::interval(std::time::Duration::from_millis(P4_TICK_MS));

    loop {
        interval.tick().await;

        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            if nodes[0].at_population_cap() {
                eprintln!("[Server] connection from {addr} refused: Sector 0 at population cap ({} ships)",
                    nodes[0].ship_count());
                drop(stream);
                continue;
            }
            let player_id = nodes[0].next_player_id();
            let ship_id = nodes[0].spawn_player_ship_at_pub(player_id, PLAYER_SPAWN);
            let payload = match nodes[0].build_handoff_payload(ship_id, AOI_CELL_SIZE) {
                Ok(payload) => payload,
                Err(error) => {
                    eprintln!("[Server] clustered fresh handshake from {addr} refused: {error}");
                    nodes[0].despawn_incomplete_handshake_spawn(ship_id);
                    drop(stream);
                    continue;
                }
            };
            let tx = ready_sess_tx.clone();
            player_sector.insert(player_id, 0);
            ship_player.insert(ship_id, player_id);

            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream,
                    addr,
                    player_id,
                    ship_id,
                    payload.initial_state,
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
            aoi_delivery.seed_cluster_player(&nodes, 0, sess.player_id, sess.ship_id);
            sessions.push(sess);
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
