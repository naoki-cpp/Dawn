//! Production Sector Node runtime frame orchestration.
//!
//! `main.rs` owns process wiring: config, TCP transports, and async accept
//! channels. This module owns one production Node frame: command dispatch,
//! jump proposal fallback, runtime tick stepping, outbound replication,
//! Redirect handling, and AoI delivery.

use dawn_actor::{protocol, ws_server};
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_replication::{OutboundLogPublisher, TcpReplicationTransport};
use dawn_sector::aoi::AoiSink;
use dawn_sector::node::{JumpOutcome, SimulationNode};
use dawn_sector::{aoi, transit};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc;

pub(crate) struct SectorNodeRuntime {
    sector_id: SectorId,
    aoi_cell_size: f32,
    peer_ws: HashMap<SectorId, SocketAddr>,
    sessions: Vec<ws_server::PlayerSession>,
    aoi_delivery: aoi::AoiDelivery,
    outbound_replication: OutboundLogPublisher<TcpReplicationTransport>,
}

impl SectorNodeRuntime {
    pub(crate) fn new<S: EventStore>(
        sector_id: SectorId,
        aoi_cell_size: f32,
        peer_ws: HashMap<SectorId, SocketAddr>,
        repl_transport: TcpReplicationTransport,
        event_store: &S,
    ) -> Self {
        Self {
            sector_id,
            aoi_cell_size,
            peer_ws,
            sessions: Vec::new(),
            aoi_delivery: aoi::AoiDelivery::new(),
            outbound_replication: OutboundLogPublisher::from_store_tail(
                repl_transport,
                event_store,
            ),
        }
    }

    pub(crate) fn promote_ready_session<S: EventStore>(
        &mut self,
        node: &SimulationNode<S>,
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
        self.aoi_delivery.seed_player(sess.player_id, seed);
        self.sessions.push(sess);
    }

    pub(crate) fn run_frame<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let event_cursor = node.total_event_count() as u64;
        let (lock_commands, pending_jumps) = self.collect_player_commands(node);

        self.propose_player_jumps(node, raft, pending_jumps);
        self.propose_auto_jumps(node, raft);

        transit::step_cluster_node(node, raft, committed_rx, &lock_commands);

        let new_events = self.collect_new_events(node, event_cursor);
        self.outbound_replication
            .publish_new_events(self.sector_id, node.event_store());

        let jumped_ships = self.jumped_ships(&new_events);
        self.deliver_frames(node, &new_events, &jumped_ships);
    }

    fn collect_player_commands<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
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

    fn propose_player_jumps<S: EventStore>(
        &self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        pending_jumps: Vec<(usize, dawn_core::JumpCommand)>,
    ) {
        for (idx, j) in pending_jumps {
            let Some(sess) = self.sessions.get(idx) else {
                continue;
            };
            if j.ship_id != sess.ship_id {
                continue;
            }
            // The in-range/auto-warp/approach fallback chain is owned by
            // dawn-sector (node/jump.rs) -- this just runs it and reacts to
            // the outcome. Only the Raft proposal itself stays here, since
            // RaftActorHandle isn't available to dawn-sector.
            match node.apply_jump_with_fallback(j.ship_id, j.gate_id) {
                JumpOutcome::NeedsTransitProposal { to } => {
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
                }
                JumpOutcome::WarpFallbackStarted => {
                    println!(
                        "[Node] Jump: ship #{} out of range - auto-warp to gate #{} started",
                        j.ship_id.raw(),
                        j.gate_id.0
                    );
                }
                JumpOutcome::ApproachFallbackStarted => {
                    println!(
                        "[Node] Jump: ship #{} too close to warp - approaching gate #{} instead",
                        j.ship_id.raw(),
                        j.gate_id.0
                    );
                }
                JumpOutcome::Rejected => {}
            }
        }
    }

    fn propose_auto_jumps<S: EventStore>(
        &self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
    ) {
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

    fn collect_new_events<S: EventStore>(
        &self,
        node: &SimulationNode<S>,
        event_cursor: u64,
    ) -> Vec<DomainEvent> {
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

    fn deliver_frames<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        new_events: &[DomainEvent],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        let grid = aoi::CellGrid::build(self.aoi_cell_size, node.ship_absolute_positions());
        let warp_arrivals = node.drain_completed_warps();
        let aoi_delivery = &mut self.aoi_delivery;

        self.sessions.retain_mut(|sess| {
            if let Some(&dest) = jumped_ships.get(&sess.ship_id) {
                if let Some(&ws_addr) = self.peer_ws.get(&dest) {
                    let msg = protocol::redirect_json(ws_addr, sess.player_id, sess.ship_id);
                    sess.conn.send_raw(&msg);
                    println!("[Node] Redirect {:?} -> {ws_addr}", sess.player_id);
                }
                aoi_delivery.retain_players(|pid| pid != sess.player_id);
                return false;
            }

            let curr = node
                .ship_absolute_pos(sess.ship_id)
                .map(|pos| grid.neighbors_of(pos))
                .unwrap_or_default();
            let observer = aoi::Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };
            let mut sink = SessionSink(sess);
            aoi_delivery.deliver_frame(&mut sink, node, observer, curr, new_events, &warp_arrivals)
        });
        let live: std::collections::HashSet<_> =
            self.sessions.iter().map(|s| s.player_id).collect();
        self.aoi_delivery.retain_players(|pid| live.contains(&pid));
    }
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// the trait lives in dawn-sector, the type in dawn-actor, so the impl has to
/// live here where both are foreign).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_raw(&mut self, msg: &str) -> bool {
        self.0.conn.send_raw(msg)
    }
    fn send_events(&mut self, events: &[DomainEvent]) -> bool {
        self.0.send_events(events)
    }
}
