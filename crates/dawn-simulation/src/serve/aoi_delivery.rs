//! Area-of-Interest delivery for serve loops.
//!
//! The serve loops own transport and runtime ordering; this module owns the
//! delivery policy for one client-visible frame: visible-set memory, AoI
//! enter/leave messages, event filtering, destroyed-ship suppression, and
//! authoritative warp-arrival snaps.

use crate::ws_server;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::node::SimulationNode;
use dawn_sector::{aoi, aoi::CellGrid};
use std::collections::{HashMap, HashSet};

pub(crate) struct AoiDelivery {
    cell_size: f32,
    visible_by_player: HashMap<PlayerId, Vec<ShipId>>,
}

impl AoiDelivery {
    pub(crate) fn new(cell_size: f32) -> Self {
        Self {
            cell_size,
            visible_by_player: HashMap::new(),
        }
    }

    pub(crate) fn seed_player(&mut self, player_id: PlayerId, visible: Vec<ShipId>) {
        self.visible_by_player.insert(player_id, visible);
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
            self.deliver_frame(sess, node, curr, new_events, warp_arrivals)
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

            self.deliver_frame(
                sess,
                &nodes[sector],
                curr,
                &new_events_by_sector[sector],
                &warp_arrivals_by_sector[sector],
            )
        });
        self.retain_sessions(sessions);
    }

    /// Push one Area-of-Interest frame to a session (ADR-0019): `AoiEnter` for
    /// ships that just became visible, `AoiLeave` for ships that left, then
    /// the new domain events that concern a currently-visible ship.
    fn deliver_frame(
        &mut self,
        sess: &mut ws_server::PlayerSession,
        node: &SimulationNode,
        curr: Vec<ShipId>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        // Ships that have a ShipDestroyed event this tick must NOT receive an
        // AoiLeave -- the client's _handle_ship_destroyed already removes them.
        let destroyed_this_tick: HashSet<ShipId> = new_events
            .iter()
            .filter_map(|e| {
                if let DomainEvent::ShipDestroyed(d) = e {
                    Some(d.ship_id)
                } else {
                    None
                }
            })
            .collect();

        let old_prev = self
            .visible_by_player
            .get(&sess.player_id)
            .cloned()
            .unwrap_or_default();
        let (entered, left) = aoi::aoi_delta(&old_prev, &curr);
        self.seed_player(sess.player_id, curr.clone());

        for id in entered.iter().filter(|&&id| id != sess.ship_id) {
            if let Some(msg) = node.aoi_enter_json(*id) {
                if !sess.conn.send_raw(&msg) {
                    return false;
                }
            }
        }
        for id in left
            .iter()
            .filter(|&&id| id != sess.ship_id && !destroyed_this_tick.contains(&id))
        {
            if !sess.conn.send_raw(&aoi::aoi_leave_json(*id)) {
                return false;
            }
        }

        let visible_events: Vec<_> = new_events
            .iter()
            .filter(|e| {
                if let DomainEvent::ShipDestroyed(d) = e {
                    return old_prev.binary_search(&d.ship_id).is_ok()
                        || old_prev.binary_search(&d.killer_id).is_ok()
                        || aoi::event_visible_to(e, &curr);
                }
                aoi::event_visible_to(e, &curr)
            })
            .cloned()
            .collect();
        if !sess.send_events(&visible_events) {
            return false;
        }

        // Warp-arrival authority (ADR-0029): a warp ends with the client's
        // visual ship lagging behind. Push the server's absolute arrival as a
        // `PositionSnap` to the owner and any observer that can see the ship.
        for &sid in warp_arrivals {
            let relevant = sid == sess.ship_id || curr.binary_search(&sid).is_ok();
            if relevant {
                if let Some(abs) = node.ship_absolute(sid) {
                    let msg = format!(
                        "{{\"type\":\"PositionSnap\",\"ship_id\":{},\"position\":{{\"x\":{},\"y\":{},\"z\":{}}}}}",
                        sid.raw(), abs[0], abs[1], abs[2]
                    );
                    if !sess.conn.send_raw(&msg) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn retain_sessions(&mut self, sessions: &[ws_server::PlayerSession]) {
        self.visible_by_player
            .retain(|pid, _| sessions.iter().any(|s| s.player_id == *pid));
    }
}

fn current_visible(node: &SimulationNode, grid: &CellGrid, ship_id: ShipId) -> Vec<ShipId> {
    node.ship_absolute_pos(ship_id)
        .map(|pos| grid.neighbors_of(pos))
        .unwrap_or_default()
}
