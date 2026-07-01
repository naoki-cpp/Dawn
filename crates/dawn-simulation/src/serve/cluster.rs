//! Raft-cluster WebSocket server (`--serve --cluster`, ADR-0009/0014).

use super::{
    build_serve_node, runtime, spawn_npc_frigates, AoiDelivery, AOI_CELL_SIZE, P4_TICK_MS,
};
use crate::{cluster, ws_server};
use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId};
use dawn_sector::node::{ClientCommandFollowup, JumpOutcome, SimulationNode};
use dawn_sector::transit;
use std::collections::HashMap;
use tokio::sync::mpsc;

pub(crate) async fn run_cluster_server(ship_count: usize, pop_cap: usize) {
    use transit::TransitOp;

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

    spawn_npc_frigates(&mut nodes[0], ship_count);

    // Warm up: tick until a Raft leader is elected (election timeout ≤ 20 ticks).
    for _ in 0..30 {
        for i in 0..SECTORS {
            let _ =
                transit::step_cluster_node(&mut nodes[i], &rafts[i], &mut committed_rxs[i], &[]);
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
            let initial_state = match nodes[0].ship_absolute_pos(ship_id) {
                Some(pos) => nodes[0].build_initial_state_json_for(pos, AOI_CELL_SIZE),
                None => nodes[0].build_initial_state_json(),
            };
            let player_fitting = nodes[0].build_player_fitting_json(ship_id);
            let tx = ready_sess_tx.clone();
            player_sector.insert(player_id, 0);
            ship_player.insert(ship_id, player_id);

            tokio::spawn(async move {
                match ws_server::WsServer::handshake(
                    stream,
                    addr,
                    player_id,
                    ship_id,
                    &initial_state,
                    player_fitting,
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
            let seed = nodes[0]
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| nodes[0].ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            aoi_delivery.seed_player(sess.player_id, seed);
            sessions.push(sess);
        }

        let mut lock_commands: Vec<Vec<dawn_core::LockOnCommand>> = vec![Vec::new(); SECTORS];

        for sess in sessions.iter_mut() {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            while let Some(cmd) = sess.try_recv_command() {
                let followup = nodes[sector].apply_client_command(
                    sess.player_id,
                    cmd,
                    &mut lock_commands[sector],
                );
                let j = match followup {
                    Some(ClientCommandFollowup::Jump(j)) => j,
                    Some(ClientCommandFollowup::RefreshFitting(ship_id)) => {
                        if let Some(json) = nodes[sector].build_player_fitting_json(ship_id) {
                            sess.send_raw(&json);
                        }
                        continue;
                    }
                    None => continue,
                };
                if j.ship_id != sess.ship_id {
                    continue;
                }
                // Fallback chain (in-range propose / auto-warp / approach) is
                // owned by dawn-sector (node/jump.rs); only the Raft proposal
                // for the in-range case stays here.
                match nodes[sector].apply_jump_with_fallback(j.ship_id, j.gate_id) {
                    JumpOutcome::NeedsTransitProposal { to } => {
                        rafts[sector].propose(
                            TransitOp::Request {
                                ship_id: j.ship_id,
                                to,
                                gate_id: Some(j.gate_id),
                            }
                            .encode(),
                        );
                        println!(
                            "  [Server] Jump proposed: ship #{} gate #{} (S{} → S{})",
                            j.ship_id.raw(),
                            j.gate_id.0,
                            sector,
                            to.0
                        );
                    }
                    JumpOutcome::WarpFallbackStarted => {
                        println!(
                            "  [Server] Jump: ship #{} out of range — auto-warp to gate #{} started",
                            j.ship_id.raw(),
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
                            j.ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                    JumpOutcome::Rejected => {
                        eprintln!(
                            "[Server] JumpCommand rejected (ship #{} gate #{})",
                            j.ship_id.raw(),
                            j.gate_id.0
                        );
                    }
                }
            }
        }

        runtime::run_cluster_runtime_tick(
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
    }
}
