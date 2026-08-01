//! Area-of-Interest session adapters for serve loops.
//!
//! Spatial-index construction, observer visible-set resolution, Enter/Leave
//! changes, event filtering, and ordered frame delivery all live in
//! `dawn_sector::aoi_frame::AoiFrame`. This module keeps only the runtime-owned
//! session loop, Sector routing, and the orphan-rule sink adapter for
//! `ws_server::PlayerSession`.

use crate::ws_server;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::aoi::{AoiSink, Observer};
use dawn_sector::aoi_frame::AoiFrame;
use dawn_sector::node::SimulationNode;
use dawn_wire::ServerMessage;
use std::collections::{HashMap, HashSet};

pub(crate) struct AoiDelivery {
    cell_size: f64,
    frames: Vec<AoiFrame>,
}

impl AoiDelivery {
    pub(crate) fn new(cell_size: f64) -> Self {
        Self {
            cell_size,
            frames: vec![AoiFrame::new(cell_size)],
        }
    }

    pub(crate) fn seed_single_player(
        &mut self,
        node: &SimulationNode,
        player_id: PlayerId,
        ship_id: ShipId,
    ) {
        self.frames[0].seed_observer(node, Observer { player_id, ship_id });
    }

    pub(crate) fn seed_cluster_player(
        &mut self,
        nodes: &[SimulationNode],
        sector: usize,
        player_id: PlayerId,
        ship_id: ShipId,
    ) {
        self.ensure_frame_count(nodes.len());
        self.frames[sector].seed_observer(&nodes[sector], Observer { player_id, ship_id });
    }

    pub(crate) fn deliver_single_sector(
        &mut self,
        node: &SimulationNode,
        sessions: &mut Vec<ws_server::PlayerSession>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) {
        let frame = &mut self.frames[0];
        frame.rebuild(node);
        sessions.retain_mut(|sess| {
            let observer = Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };
            let mut sink = SessionSink(sess);
            frame.deliver_observer(&mut sink, node, observer, new_events, warp_arrivals)
        });

        let live: HashSet<PlayerId> = sessions.iter().map(|session| session.player_id).collect();
        frame.retain_players(|player_id| live.contains(&player_id));
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
        self.ensure_frame_count(nodes.len());
        for (frame, node) in self.frames.iter_mut().zip(nodes) {
            frame.rebuild(node);
        }

        sessions.retain_mut(|sess| {
            let sector = *player_sector.get(&sess.player_id).unwrap_or(&0);
            let observer = Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };

            if reseed_players.contains(&sess.player_id) {
                self.frames[sector].seed_observer_from_index(&nodes[sector], observer);
                return true;
            }

            let mut sink = SessionSink(sess);
            self.frames[sector].deliver_observer(
                &mut sink,
                &nodes[sector],
                observer,
                &new_events_by_sector[sector],
                &warp_arrivals_by_sector[sector],
            )
        });

        let live: HashSet<PlayerId> = sessions.iter().map(|session| session.player_id).collect();
        for (sector, frame) in self.frames.iter_mut().enumerate() {
            frame.retain_players(|player_id| {
                live.contains(&player_id) && player_sector.get(&player_id) == Some(&sector)
            });
        }
    }

    fn ensure_frame_count(&mut self, count: usize) {
        let cell_size = self.cell_size;
        self.frames
            .resize_with(count, || AoiFrame::new(cell_size));
    }
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// the concrete session type and the trait live in different crates).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_events(&mut self, events: &[DomainEvent]) -> bool {
        self.0.send_events(events)
    }

    fn send_message(&mut self, msg: &ServerMessage) -> bool {
        self.0.send_message(msg)
    }
}
