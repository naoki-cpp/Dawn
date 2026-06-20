//! Raft-cluster WebSocket server (`--serve --cluster`, ADR-0009/0014).

use super::{AOI_CELL_SIZE, P4_TICK_MS, apply_common_command, build_serve_node, deliver_aoi_frame, spawn_npc_frigates};
use crate::{cluster, ws_server};
use dawn_sector::{aoi, transit};
use dawn_sector::node::SimulationNode;
use dawn_core::{DomainEvent, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, WarpTarget};
use std::collections::HashMap;
use tokio::sync::mpsc;

pub(crate) async fn run_cluster_server(ship_count: usize, pop_cap: usize) {
    use dawn_event_store::store::EventStore as _;
    use transit::TransitOp;

    const SECTORS: usize = 3;
    /// 2x the Alpha star (Helios) radius from Sector origin (matches
    /// SimulationNode::DEFAULT_PLAYER_SPAWN): clear of the star body itself,
    /// short of Gate 0's activation radius (49,000±2,000), and well beyond
    /// the 3,000u warp minimum, so warp/approach to the gate both work
    /// (ADR-0022).
    const PLAYER_SPAWN: Position = Position { x: 30_000.0, y: 0.0, z: 0.0 };

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

    let server = ws_server::WsServer::bind("127.0.0.1:7878").await
        .expect("failed to bind WebSocket server");

    let ids: Vec<NodeId> = (0..SECTORS as u8).map(NodeId).collect();
    let (endpoints, _partitioned) = cluster::spawn_raft_actors(&ids);
    let (rafts, mut committed_rxs): (Vec<_>, Vec<_>) = endpoints.into_iter().unzip();

    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut nodes: Vec<SimulationNode> = ids.iter()
        .map(|&id| build_serve_node(id, SectorId(id.0), bounds, pop_cap))
        .collect();

    spawn_npc_frigates(&mut nodes[0], ship_count);

    // Warm up: tick until a Raft leader is elected (election timeout ≤ 20 ticks).
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
    let mut player_sector: HashMap<PlayerId, usize> = HashMap::new();
    let mut ship_player  : HashMap<ShipId, PlayerId> = HashMap::new();
    let mut prev_visible : HashMap<PlayerId, Vec<ShipId>> = HashMap::new();

    let mut interval = tokio::time::interval(
        std::time::Duration::from_millis(P4_TICK_MS)
    );

    loop {
        interval.tick().await;

        while let Ok((stream, addr)) = new_conn_rx.try_recv() {
            if nodes[0].at_population_cap() {
                eprintln!("[Server] connection from {addr} refused: Sector 0 at population cap ({} ships)",
                    nodes[0].ship_count());
                drop(stream);
                continue;
            }
            let player_id      = nodes[0].next_player_id();
            let ship_id        = nodes[0].spawn_player_ship_at_pub(player_id, PLAYER_SPAWN);
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
            let seed = nodes[0].get_ship_position(sess.ship_id)
                .map(|pos| nodes[0].ships_visible_to(pos, AOI_CELL_SIZE))
                .unwrap_or_default();
            prev_visible.insert(sess.player_id, seed);
            sessions.push(sess);
        }

        let events_before: Vec<u64> =
            nodes.iter().map(|n| n.total_event_count() as u64).collect();

        let mut lock_commands: Vec<Vec<dawn_core::LockOnCommand>> =
            vec![Vec::new(); SECTORS];

        for sess in sessions.iter_mut() {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            while let Some(cmd) = sess.try_recv_command() {
                let Some(j) = apply_common_command(&mut nodes[sector], sess.player_id, cmd, &mut lock_commands[sector])
                else { continue };
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
                    println!("  [Server] Jump: ship #{} out of range — auto-warp to gate #{} started",
                        j.ship_id.raw(), j.gate_id.0);
                } else {
                    eprintln!("[Server] JumpCommand rejected (ship #{} gate #{})",
                        j.ship_id.raw(), j.gate_id.0);
                }
            }
        }

        for i in 0..SECTORS {
            transit::step_cluster_node(&mut nodes[i], &rafts[i], &mut committed_rxs[i], &lock_commands[i]);
        }

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

        let events_by_sector: Vec<Vec<DomainEvent>> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                node.event_store().iter_from(events_before[i]).map(|r| r.event.clone()).collect()
            })
            .collect();

        // Ownership handoff: a player ship completed a jump.
        let mut jumped_players: Vec<(PlayerId, usize)> = Vec::new();
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

            if jumped_ids.contains(&sess.player_id) {
                prev_visible.insert(sess.player_id, curr);
                return true;
            }

            let prev = prev_visible.entry(sess.player_id).or_default();
            deliver_aoi_frame(sess, &nodes[sector], curr, prev, &events_by_sector[sector])
        });
        prev_visible.retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));

        // Resend scoped InitialState to players that just jumped.
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
