//! Serve runtime orchestration shared by clustered serve loops.

use super::{deliver_aoi_frame, AOI_CELL_SIZE};
use crate::ws_server;
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::node::SimulationNode;
use dawn_sector::{aoi, transit};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

/// Run one clustered serve tick and deliver the resulting client frames.
///
/// This module owns the ordering around runtime tick outputs: auto-jump
/// proposals, jump ownership handoff, AOI delivery, and scoped InitialState
/// resend for players that changed Sector.
pub(crate) struct ClusterRuntimeTickContext<'a> {
    pub(crate) nodes: &'a mut [SimulationNode],
    pub(crate) rafts: &'a [RaftActorHandle],
    pub(crate) committed_rxs: &'a mut [mpsc::UnboundedReceiver<Vec<u8>>],
    pub(crate) sessions: &'a mut Vec<ws_server::PlayerSession>,
    pub(crate) player_sector: &'a mut HashMap<PlayerId, usize>,
    pub(crate) ship_player: &'a HashMap<ShipId, PlayerId>,
    pub(crate) prev_visible: &'a mut HashMap<PlayerId, Vec<ShipId>>,
}

pub(crate) fn run_cluster_runtime_tick(
    ctx: ClusterRuntimeTickContext<'_>,
    lock_commands: &[Vec<dawn_core::LockOnCommand>],
) {
    let tick_outputs: Vec<_> = (0..ctx.nodes.len())
        .map(|i| {
            transit::run_runtime_tick(
                &mut ctx.nodes[i],
                &ctx.rafts[i],
                &mut ctx.committed_rxs[i],
                &lock_commands[i],
                |_, _| {},
            )
        })
        .collect();

    propose_auto_jumps(ctx.nodes, ctx.rafts, &tick_outputs);

    let warp_arrivals_by_sector: Vec<Vec<ShipId>> = tick_outputs
        .iter()
        .map(|output| output.completed_warps.clone())
        .collect();
    let events_by_sector: Vec<Vec<DomainEvent>> = tick_outputs
        .iter()
        .map(|output| output.events.clone())
        .collect();

    let handoff = apply_jump_handoffs(
        ctx.nodes,
        ctx.player_sector,
        ctx.ship_player,
        &events_by_sector,
    );
    deliver_cluster_frames(
        ctx.nodes,
        ctx.sessions,
        ctx.player_sector,
        ctx.prev_visible,
        &events_by_sector,
        &warp_arrivals_by_sector,
        &handoff,
    );
    resend_jump_initial_state(ctx.nodes, ctx.sessions, &handoff);
}

struct JumpHandoff {
    jumped_players: Vec<(PlayerId, usize)>,
    own_events: HashMap<PlayerId, Vec<DomainEvent>>,
}

fn propose_auto_jumps(
    nodes: &[SimulationNode],
    rafts: &[RaftActorHandle],
    tick_outputs: &[transit::RuntimeTickOutput],
) {
    for (i, output) in tick_outputs.iter().enumerate() {
        for (ship_id, gate_id) in &output.pending_auto_jumps {
            if nodes[i].can_propose_jump(*ship_id, *gate_id) {
                let to = nodes[i]
                    .jump_gate(*gate_id)
                    .expect("gate must exist if can_propose_jump passed")
                    .to_sector;
                rafts[i].propose(
                    transit::TransitOp::Request {
                        ship_id: *ship_id,
                        to,
                        gate_id: Some(*gate_id),
                    }
                    .encode(),
                );
                println!(
                    "  [Server] Auto-jump proposed: ship #{} gate #{} (S{} -> S{})",
                    ship_id.raw(),
                    gate_id.0,
                    i,
                    to.0
                );
            }
        }
    }
}

fn apply_jump_handoffs(
    nodes: &mut [SimulationNode],
    player_sector: &mut HashMap<PlayerId, usize>,
    ship_player: &HashMap<ShipId, PlayerId>,
    events_by_sector: &[Vec<DomainEvent>],
) -> JumpHandoff {
    let mut jumped_players: Vec<(PlayerId, usize)> = Vec::new();
    let mut own_events: HashMap<PlayerId, Vec<DomainEvent>> = HashMap::new();

    for sector_events in events_by_sector {
        for event in sector_events {
            match event {
                DomainEvent::JumpGateUsed(e) => {
                    if let Some(&player_id) = ship_player.get(&e.ship_id) {
                        let dest = e.to_sector.0 as usize;
                        nodes[dest].adopt_player_ship(e.ship_id, player_id);
                        player_sector.insert(player_id, dest);
                        jumped_players.push((player_id, dest));
                        own_events.entry(player_id).or_default().push(event.clone());
                        println!(
                            "  [Server] {player_id:?} ship #{} now owned by Sector {dest}",
                            e.ship_id.raw()
                        );
                    }
                }
                DomainEvent::StarSystemChanged(e) => {
                    if let Some(&player_id) = ship_player.get(&e.ship_id) {
                        own_events.entry(player_id).or_default().push(event.clone());
                    }
                }
                _ => {}
            }
        }
    }

    JumpHandoff {
        jumped_players,
        own_events,
    }
}

fn deliver_cluster_frames(
    nodes: &[SimulationNode],
    sessions: &mut Vec<ws_server::PlayerSession>,
    player_sector: &HashMap<PlayerId, usize>,
    prev_visible: &mut HashMap<PlayerId, Vec<ShipId>>,
    events_by_sector: &[Vec<DomainEvent>],
    warp_arrivals_by_sector: &[Vec<ShipId>],
    handoff: &JumpHandoff,
) {
    let grids: Vec<aoi::CellGrid> = nodes
        .iter()
        .map(|n| aoi::CellGrid::build(AOI_CELL_SIZE, n.ship_absolute_positions()))
        .collect();
    let jumped_ids: HashSet<PlayerId> = handoff
        .jumped_players
        .iter()
        .map(|(player_id, _)| *player_id)
        .collect();

    sessions.retain_mut(|sess| {
        let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
        let curr = nodes[sector]
            .ship_absolute_pos(sess.ship_id)
            .map(|pos| grids[sector].neighbors_of(pos))
            .unwrap_or_default();

        if jumped_ids.contains(&sess.player_id) {
            prev_visible.insert(sess.player_id, curr);
            return true;
        }

        let prev = prev_visible.entry(sess.player_id).or_default();
        deliver_aoi_frame(
            sess,
            &nodes[sector],
            curr,
            prev,
            &events_by_sector[sector],
            &warp_arrivals_by_sector[sector],
        )
    });
    prev_visible.retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));
}

fn resend_jump_initial_state(
    nodes: &[SimulationNode],
    sessions: &[ws_server::PlayerSession],
    handoff: &JumpHandoff,
) {
    for (player_id, dest) in &handoff.jumped_players {
        if let Some(sess) = sessions.iter().find(|s| s.player_id == *player_id) {
            if let Some(events) = handoff.own_events.get(player_id) {
                sess.send_events(events);
            }
            let initial_state = nodes[*dest]
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| nodes[*dest].build_initial_state_json_for(pos, AOI_CELL_SIZE))
                .unwrap_or_else(|| nodes[*dest].build_initial_state_json());
            sess.conn.send_raw(&initial_state);
        }
    }
}
