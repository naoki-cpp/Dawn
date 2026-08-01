//! Production Sector Node runtime frame orchestration.
//!
//! `main.rs` owns process wiring: config, TCP transports, and async accept
//! channels. This module owns one production Node frame: command dispatch,
//! jump proposal fallback, runtime tick stepping, outbound replication,
//! Redirect handling, and AoI delivery.

use dawn_actor::ws_server;
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_replication::{OutboundLogPublisher, TcpReplicationTransport};
use dawn_sector::aoi::{AoiSink, Observer};
use dawn_sector::aoi_frame::AoiFrame;
use dawn_sector::node::{ClientCommandFollowup, JumpOutcome, SimulationNode};
use dawn_sector::transit;
use dawn_wire::ServerMessage;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::mpsc;

pub(crate) struct SectorNodeRuntime {
    sector_id: SectorId,
    peer_ws: HashMap<SectorId, SocketAddr>,
    sessions: Vec<ws_server::PlayerSession>,
    aoi_frame: AoiFrame,
    outbound_replication: OutboundLogPublisher<TcpReplicationTransport>,
}

impl SectorNodeRuntime {
    pub(crate) fn new<S: EventStore>(
        sector_id: SectorId,
        aoi_cell_size: f64,
        peer_ws: HashMap<SectorId, SocketAddr>,
        repl_transport: TcpReplicationTransport,
        event_store: &S,
    ) -> Self {
        Self {
            sector_id,
            peer_ws,
            sessions: Vec::new(),
            aoi_frame: AoiFrame::new(aoi_cell_size),
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
        self.aoi_frame.seed_observer(
            node,
            Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            },
        );
        self.sessions.push(sess);
    }

    pub(crate) fn run_frame<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let (lock_commands, pending_jumps) = self.collect_player_commands(node);
        self.propose_player_jumps(node, raft, pending_jumps);

        let sector_id = self.sector_id;
        let outbound_replication = &mut self.outbound_replication;
        let output =
            transit::run_runtime_tick(node, raft, committed_rx, &lock_commands, |node, _, _| {
                outbound_replication.publish_new_events(sector_id, node.event_store());
            });

        self.log_auto_jumps(&output.pending_auto_jumps);
        let jumped_ships = self.jumped_ships(&output.events);
        self.deliver_frames(node, &output.events, &output.completed_warps, &jumped_ships);
    }

    fn collect_player_commands<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
    ) -> (
        Vec<dawn_core::LockOnCommand>,
        Vec<(usize, ShipId, dawn_core::JumpCommand)>,
    ) {
        let mut lock_commands = Vec::new();
        let mut pending_jumps = Vec::new();

        for (i, sess) in self.sessions.iter_mut().enumerate() {
            while let Some(cmd) = sess.try_recv_command() {
                match node.apply_client_command(sess.player_id, cmd, &mut lock_commands) {
                    Some(ClientCommandFollowup::Jump { ship_id, command }) => {
                        pending_jumps.push((i, ship_id, command));
                        break;
                    }
                    Some(followup @ ClientCommandFollowup::RefreshPlayerLoadout { .. }) => {
                        if let Some(player_id) = followup.loadout_player_id() {
                            if let Some(loadout) =
                                node.build_player_loadout_json_for_player(player_id)
                            {
                                sess.send_message(&ServerMessage::PlayerLoadout(loadout));
                            }
                        }
                    }
                    None => {}
                }
            }
        }

        (lock_commands, pending_jumps)
    }

    fn propose_player_jumps<S: EventStore>(
        &self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        pending_jumps: Vec<(usize, ShipId, dawn_core::JumpCommand)>,
    ) {
        for (idx, ship_id, j) in pending_jumps {
            let Some(sess) = self.sessions.get(idx) else {
                continue;
            };
            if ship_id != sess.ship_id {
                continue;
            }
            match transit::propose_jump(node, raft, ship_id, j.gate_id) {
                JumpOutcome::NeedsTransitProposal { to } => {
                    println!(
                        "[Node] Jump proposed: ship #{} gate #{} (-> S{})",
                        ship_id.raw(),
                        j.gate_id.0,
                        to.0
                    );
                }
                JumpOutcome::WarpFallbackStarted => {
                    println!(
                        "[Node] Jump: ship #{} out of range - auto-warp to gate #{} started",
                        ship_id.raw(),
                        j.gate_id.0
                    );
                }
                JumpOutcome::ApproachFallbackStarted => {
                    println!(
                        "[Node] Jump: ship #{} too close to warp - approaching gate #{} instead",
                        ship_id.raw(),
                        j.gate_id.0
                    );
                }
                JumpOutcome::Rejected => {}
            }
        }
    }

    fn log_auto_jumps(&self, auto_jumps: &[(ShipId, dawn_core::JumpGateId)]) {
        for (ship_id, gate_id) in auto_jumps {
            println!(
                "[Node] Auto-jump proposed: ship #{} gate #{}",
                ship_id.raw(),
                gate_id.0
            );
        }
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
        node: &SimulationNode<S>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        self.aoi_frame.rebuild(node);
        let aoi_frame = &mut self.aoi_frame;
        let peer_ws = &self.peer_ws;

        self.sessions.retain_mut(|sess| {
            if let Some(&dest) = jumped_ships.get(&sess.ship_id) {
                if let Some(&ws_addr) = peer_ws.get(&dest) {
                    sess.conn.send_message(&ServerMessage::Redirect {
                        ws_addr: ws_addr.to_string(),
                        player_id: sess.player_id.raw(),
                        ship_id: sess.ship_id.raw(),
                    });
                    println!("[Node] Redirect {:?} -> {ws_addr}", sess.player_id);
                }
                aoi_frame.retain_players(|player_id| player_id != sess.player_id);
                return false;
            }

            let observer = Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            };
            let mut sink = SessionSink(sess);
            aoi_frame.deliver_observer(
                &mut sink,
                node,
                observer,
                new_events,
                warp_arrivals,
            )
        });
        let live: std::collections::HashSet<_> =
            self.sessions.iter().map(|session| session.player_id).collect();
        self.aoi_frame
            .retain_players(|player_id| live.contains(&player_id));
    }
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// the trait lives in dawn-sector, the type in dawn-actor, so the impl has to
/// live here where both are foreign).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_events(&mut self, events: &[DomainEvent]) -> bool {
        self.0.send_events(events)
    }

    fn send_message(&mut self, msg: &ServerMessage) -> bool {
        self.0.send_message(msg)
    }
}
