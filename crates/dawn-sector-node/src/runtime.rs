//! Production Sector Node runtime frame orchestration.
//!
//! `main.rs` owns process wiring: config, TCP transports, and async accept
//! channels. This module owns one production Node frame: command dispatch,
//! jump proposal fallback, runtime tick stepping, outbound replication,
//! Redirect handling, and AoI delivery.

use dawn_actor::{protocol, ws_server};
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, PlayerId, SectorId, ShipId, WarpTarget};
use dawn_event_store::store::EventStore as _;
use dawn_replication::{LogBatch, ReplicationTransport, TcpReplicationTransport};
use dawn_sector::node::SimulationNode;
use dawn_sector::{aoi, transit};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc;

pub(crate) struct SectorNodeRuntime {
    sector_id: SectorId,
    aoi_cell_size: f32,
    peer_ws: HashMap<SectorId, SocketAddr>,
    sessions: Vec<ws_server::PlayerSession>,
    prev_visible: HashMap<PlayerId, Vec<ShipId>>,
}

impl SectorNodeRuntime {
    pub(crate) fn new(
        sector_id: SectorId,
        aoi_cell_size: f32,
        peer_ws: HashMap<SectorId, SocketAddr>,
    ) -> Self {
        Self {
            sector_id,
            aoi_cell_size,
            peer_ws,
            sessions: Vec::new(),
            prev_visible: HashMap::new(),
        }
    }

    pub(crate) fn promote_ready_session(
        &mut self,
        node: &SimulationNode,
        sess: ws_server::PlayerSession,
    ) {
        println!(
            "[Node] {:?} joined with ship #{}",
            sess.player_id,
            sess.ship_id.raw()
        );
        let seed = node
            .ship_absolute_pos(sess.ship_id)
            .map(|pos| node.ships_visible_to(pos, self.aoi_cell_size))
            .unwrap_or_default();
        self.prev_visible.insert(sess.player_id, seed);
        self.sessions.push(sess);
    }

    pub(crate) fn run_frame(
        &mut self,
        node: &mut SimulationNode,
        raft: &RaftActorHandle,
        committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
        repl_transport: &TcpReplicationTransport,
    ) {
        let event_cursor = node.total_event_count() as u64;
        let (lock_commands, pending_jumps) = self.collect_player_commands(node);

        self.propose_player_jumps(node, raft, pending_jumps);
        self.propose_auto_jumps(node, raft);

        transit::step_cluster_node(node, raft, committed_rx, &lock_commands);

        let new_events = self.collect_new_events(node, event_cursor);
        if !new_events.is_empty() {
            let batch = LogBatch::new(self.sector_id, event_cursor, new_events.clone());
            repl_transport.broadcast(batch);
        }

        let jumped_ships = self.jumped_ships(&new_events);
        self.deliver_frames(node, &new_events, &jumped_ships);
    }

    fn collect_player_commands(
        &mut self,
        node: &mut SimulationNode,
    ) -> (
        Vec<dawn_core::LockOnCommand>,
        Vec<(usize, dawn_core::JumpCommand)>,
    ) {
        let mut lock_commands = Vec::new();
        let mut pending_jumps = Vec::new();

        for (i, sess) in self.sessions.iter_mut().enumerate() {
            while let Some(cmd) = sess.try_recv_command() {
                match cmd {
                    dawn_actor::ClientCommand::Move(mv) => {
                        node.apply_move_command_owned(
                            sess.player_id,
                            mv.ship_id,
                            mv.target_position,
                        );
                    }
                    dawn_actor::ClientCommand::LockOn(lo) => {
                        if node.owns_ship(sess.player_id, lo.ship_id) {
                            lock_commands.push(lo);
                        }
                    }
                    dawn_actor::ClientCommand::Activate(c) => {
                        node.activate_module_owned(sess.player_id, c);
                    }
                    dawn_actor::ClientCommand::Deactivate(c) => {
                        node.deactivate_module_owned(sess.player_id, c);
                    }
                    dawn_actor::ClientCommand::Attack(_) => {}
                    dawn_actor::ClientCommand::Stop(s) => {
                        node.apply_stop_command_owned(sess.player_id, s.ship_id);
                    }
                    dawn_actor::ClientCommand::Approach(a) => {
                        node.apply_approach_command_owned(sess.player_id, a);
                    }
                    dawn_actor::ClientCommand::Warp(w) => {
                        node.apply_warp_command_owned(sess.player_id, w);
                    }
                    dawn_actor::ClientCommand::Orbit(o) => {
                        node.apply_orbit_command_owned(sess.player_id, o);
                    }
                    dawn_actor::ClientCommand::KeepAtRange(k) => {
                        node.apply_keep_at_range_command_owned(sess.player_id, k);
                    }
                    dawn_actor::ClientCommand::Fit(f) => {
                        let ship_id = f.ship_id;
                        node.fit_module_owned(sess.player_id, f);
                        if let Some(json) = node.build_player_fitting_json(ship_id) {
                            sess.send_raw(&json);
                        }
                    }
                    dawn_actor::ClientCommand::Unfit(u) => {
                        let ship_id = u.ship_id;
                        node.unfit_module_owned(sess.player_id, u);
                        if let Some(json) = node.build_player_fitting_json(ship_id) {
                            sess.send_raw(&json);
                        }
                    }
                    dawn_actor::ClientCommand::Jump(j) => {
                        pending_jumps.push((i, j));
                        break;
                    }
                }
            }
        }

        (lock_commands, pending_jumps)
    }

    fn propose_player_jumps(
        &self,
        node: &mut SimulationNode,
        raft: &RaftActorHandle,
        pending_jumps: Vec<(usize, dawn_core::JumpCommand)>,
    ) {
        for (idx, j) in pending_jumps {
            let Some(sess) = self.sessions.get(idx) else {
                continue;
            };
            let ship_owned = j.ship_id == sess.ship_id;
            let in_range = ship_owned && node.can_propose_jump(j.ship_id, j.gate_id);
            if in_range {
                let to = node
                    .jump_gate(j.gate_id)
                    .expect("gate must exist")
                    .to_sector;
                raft.propose(
                    transit::TransitOp::Request {
                        ship_id: j.ship_id,
                        to,
                        gate_id: Some(j.gate_id),
                    }
                    .encode(),
                );
                println!(
                    "[Node] Jump proposed: ship #{} gate #{}",
                    j.ship_id.raw(),
                    j.gate_id.0
                );
            } else if ship_owned
                && node.apply_warp_command(j.ship_id, WarpTarget::Gate(j.gate_id), true)
            {
                println!(
                    "[Node] Jump: ship #{} out of range - auto-warp to gate #{} started",
                    j.ship_id.raw(),
                    j.gate_id.0
                );
            } else if ship_owned
                && node
                    .apply_approach_command(j.ship_id, dawn_core::ApproachTarget::Gate(j.gate_id))
            {
                println!(
                    "[Node] Jump: ship #{} too close to warp - approaching gate #{} instead",
                    j.ship_id.raw(),
                    j.gate_id.0
                );
            }
        }
    }

    fn propose_auto_jumps(&self, node: &mut SimulationNode, raft: &RaftActorHandle) {
        for (ship_id, gate_id) in node.drain_pending_auto_jumps() {
            if node.can_propose_jump(ship_id, gate_id) {
                let to = node.jump_gate(gate_id).expect("gate must exist").to_sector;
                raft.propose(
                    transit::TransitOp::Request {
                        ship_id,
                        to,
                        gate_id: Some(gate_id),
                    }
                    .encode(),
                );
                println!(
                    "[Node] Auto-jump proposed: ship #{} gate #{}",
                    ship_id.raw(),
                    gate_id.0
                );
            }
        }
    }

    fn collect_new_events(&self, node: &SimulationNode, event_cursor: u64) -> Vec<DomainEvent> {
        node.event_store()
            .iter_from(event_cursor)
            .map(|r| r.event.clone())
            .collect()
    }

    fn jumped_ships(&self, new_events: &[DomainEvent]) -> HashMap<ShipId, SectorId> {
        new_events
            .iter()
            .filter_map(|e| {
                if let DomainEvent::JumpGateUsed(j) = e {
                    if j.from_sector == self.sector_id {
                        Some((j.ship_id, j.to_sector))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn deliver_frames(
        &mut self,
        node: &mut SimulationNode,
        new_events: &[DomainEvent],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        let grid = aoi::CellGrid::build(self.aoi_cell_size, node.ship_absolute_positions());
        let warp_arrivals = node.drain_completed_warps();

        self.sessions.retain_mut(|sess| {
            if let Some(&dest) = jumped_ships.get(&sess.ship_id) {
                if let Some(&ws_addr) = self.peer_ws.get(&dest) {
                    let msg = protocol::redirect_json(ws_addr, sess.player_id, sess.ship_id);
                    sess.conn.send_raw(&msg);
                    println!("[Node] Redirect {:?} -> {ws_addr}", sess.player_id);
                }
                self.prev_visible.remove(&sess.player_id);
                return false;
            }

            let curr = node
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| grid.neighbors_of(pos))
                .unwrap_or_default();
            let prev = self.prev_visible.entry(sess.player_id).or_default();
            deliver_aoi_frame(sess, node, curr, prev, new_events, &warp_arrivals)
        });
        self.prev_visible
            .retain(|pid, _| self.sessions.iter().any(|s| s.player_id == *pid));
    }
}

fn deliver_aoi_frame(
    sess: &mut ws_server::PlayerSession,
    node: &SimulationNode,
    curr: Vec<ShipId>,
    prev: &mut Vec<ShipId>,
    new_events: &[DomainEvent],
    warp_arrivals: &[ShipId],
) -> bool {
    let destroyed_this_tick: std::collections::HashSet<ShipId> = new_events
        .iter()
        .filter_map(|e| {
            if let DomainEvent::ShipDestroyed(d) = e {
                Some(d.ship_id)
            } else {
                None
            }
        })
        .collect();

    let old_prev = prev.clone();
    let (entered, left) = aoi::aoi_delta(&old_prev, &curr);
    *prev = curr.clone();

    for &id in entered.iter().filter(|&&id| id != sess.ship_id) {
        if let Some(msg) = node.aoi_enter_json(id) {
            if !sess.conn.send_raw(&msg) {
                return false;
            }
        }
    }
    for &id in left
        .iter()
        .filter(|&&id| id != sess.ship_id && !destroyed_this_tick.contains(&id))
    {
        if !sess.conn.send_raw(&aoi::aoi_leave_json(id)) {
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

    for &sid in warp_arrivals {
        if sid == sess.ship_id || curr.binary_search(&sid).is_ok() {
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
