//! Area-of-Interest delivery for serve loops.
//!
//! The delivery policy itself (visible-set memory, AoI enter/leave, event
//! filtering, warp-arrival snaps) lives in `dawn_sector::aoi::AoiDelivery` —
//! this module only owns what's specific to the in-process serve loop:
//! building the per-tick `CellGrid`, looping over sessions, and adapting
//! `ws_server::PlayerSession` to the `AoiSink` trait (the type and the trait
//! live in different crates, so the adapter has to live here or in
//! dawn-actor — see AI_DEVELOPMENT_GUIDE.md crate boundaries).

use crate::ws_server;
use dawn_actor::protocol::ServerMessage;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::aoi::{AoiSink, CellGrid};
use dawn_sector::node::SimulationNode;
use std::collections::{HashMap, HashSet};

pub(crate) struct AoiDelivery {
    cell_size: f64,
    inner: dawn_sector::aoi::AoiDelivery,
}

impl AoiDelivery {
    pub(crate) fn new(cell_size: f64) -> Self {
        Self {
            cell_size,
            inner: dawn_sector::aoi::AoiDelivery::new(),
        }
    }

    pub(crate) fn seed_player(&mut self, player_id: PlayerId, visible: Vec<ShipId>) {
        self.inner.seed_player(player_id, visible);
    }

    pub(crate) fn deliver_single_sector(
        &mut self,
        node: &SimulationNode,
        sessions: &mut Vec<ws_server::PlayerSession>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) {
        let grid = CellGrid::build(self.cell_size, node.ship_absolute_positions());
        sessions.retain_mut(|sess| {
            let curr = current_visible(node, &grid, sess.ship_id);
            let observer = dawn_sector::aoi::Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };
            let mut sink = SessionSink(sess);
            self.inner
                .deliver_frame(&mut sink, node, observer, curr, new_events, warp_arrivals)
        });
        self.retain_sessions(sessions);
    }

    pub(crate) fn deliver_cluster_sectors(
        &mut self,
        nodes: &[SimulationNode],
        sessions: &mut Vec<ws_server::PlayerSession>,
        player_sector: &HashMap<PlayerId, usize>,
        new_events_by_sector: &[Vec<DomainEvent>],
        warp_arrivals_by_sector: &[Vec<ShipId>],
        reseed_players: &HashSet<PlayerId>,
    ) {
        let grids: Vec<CellGrid> = nodes
            .iter()
            .map(|n| CellGrid::build(self.cell_size, n.ship_absolute_positions()))
            .collect();

        sessions.retain_mut(|sess| {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            let curr = current_visible(&nodes[sector], &grids[sector], sess.ship_id);

            if reseed_players.contains(&sess.player_id) {
                self.seed_player(sess.player_id, curr);
                return true;
            }

            let observer = dawn_sector::aoi::Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };
            let mut sink = SessionSink(sess);
            self.inner.deliver_frame(
                &mut sink,
                &nodes[sector],
                observer,
                curr,
                &new_events_by_sector[sector],
                &warp_arrivals_by_sector[sector],
            )
        });
        self.retain_sessions(sessions);
    }

    fn retain_sessions(&mut self, sessions: &[ws_server::PlayerSession]) {
        let live: HashSet<PlayerId> = sessions.iter().map(|s| s.player_id).collect();
        self.inner.retain_players(|pid| live.contains(&pid));
    }
}

fn current_visible(node: &SimulationNode, grid: &CellGrid, ship_id: ShipId) -> Vec<ShipId> {
    node.ship_absolute_pos(ship_id)
        .map(|pos| grid.neighbors_of(pos))
        .unwrap_or_default()
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// neither this crate nor dawn-actor can be skipped — the wrapper just has to
/// live wherever the concrete session type is in scope).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_events(&mut self, events: &[DomainEvent]) -> bool {
        self.0.send_events(events)
    }
    fn send_message(&mut self, msg: &ServerMessage) -> bool {
        self.0.send_message(msg)
    }
}
