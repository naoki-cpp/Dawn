//! Production Sector Node runtime frame orchestration.
//!
//! `main.rs` owns process wiring: config, TCP transports, and async accept
//! channels. This module owns one production Node frame: command dispatch,
//! jump proposal fallback, runtime tick stepping, outbound replication,
//! Redirect handling, and AoI delivery.

use dawn_actor::ws_server;
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, PlayerId, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_replication::{OutboundLogPublisher, TcpReplicationTransport};
use dawn_sector::aoi::{AoiSink, Observer};
use dawn_sector::aoi_frame::AoiFrame;
use dawn_sector::node::{ClientCommandFollowup, JumpOutcome, SimulationNode};
use dawn_sector::transit;
use dawn_wire::ServerMessage;
use std::collections::{HashMap, HashSet};
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
        self.sessions.retain(|existing| {
            existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
        });
        self.aoi_frame
            .retain_players(|player_id| player_id != sess.player_id);
        seed_runtime_session(&mut self.aoi_frame, node, &sess);
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
        deliver_runtime_sessions(
            &mut self.aoi_frame,
            node,
            &mut self.sessions,
            &self.peer_ws,
            new_events,
            warp_arrivals,
            jumped_ships,
        );
    }
}

trait RuntimeAoiSession {
    fn player_id(&self) -> PlayerId;
    fn ship_id(&self) -> ShipId;
    fn send_redirect(&mut self, ws_addr: SocketAddr);

    fn deliver<S: EventStore>(
        &mut self,
        frame: &mut AoiFrame,
        node: &SimulationNode<S>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool;
}

fn seed_runtime_session<S: EventStore, T: RuntimeAoiSession>(
    frame: &mut AoiFrame,
    node: &SimulationNode<S>,
    session: &T,
) {
    frame.seed_observer(
        node,
        Observer {
            player_id: session.player_id(),
            ship_id: session.ship_id(),
        },
    );
}

fn deliver_runtime_sessions<S: EventStore, T: RuntimeAoiSession>(
    frame: &mut AoiFrame,
    node: &SimulationNode<S>,
    sessions: &mut Vec<T>,
    peer_ws: &HashMap<SectorId, SocketAddr>,
    new_events: &[DomainEvent],
    warp_arrivals: &[ShipId],
    jumped_ships: &HashMap<ShipId, SectorId>,
) {
    frame.rebuild(node);

    sessions.retain_mut(|session| {
        if let Some(&dest) = jumped_ships.get(&session.ship_id()) {
            if let Some(&ws_addr) = peer_ws.get(&dest) {
                session.send_redirect(ws_addr);
                println!("[Node] Redirect {:?} -> {ws_addr}", session.player_id());
            }
            frame.retain_players(|player_id| player_id != session.player_id());
            return false;
        }

        session.deliver(frame, node, new_events, warp_arrivals)
    });

    let live: HashSet<PlayerId> = sessions.iter().map(RuntimeAoiSession::player_id).collect();
    frame.retain_players(|player_id| live.contains(&player_id));
}

impl RuntimeAoiSession for ws_server::PlayerSession {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn ship_id(&self) -> ShipId {
        self.ship_id
    }

    fn send_redirect(&mut self, ws_addr: SocketAddr) {
        self.conn.send_message(&ServerMessage::Redirect {
            ws_addr: ws_addr.to_string(),
            resume_ticket: self.resume_ticket,
        });
    }

    fn deliver<S: EventStore>(
        &mut self,
        frame: &mut AoiFrame,
        node: &SimulationNode<S>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let observer = Observer {
            player_id: self.player_id,
            ship_id: self.ship_id,
        };
        let mut sink = SessionSink(self);
        frame.deliver_observer(&mut sink, node, observer, new_events, warp_arrivals)
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

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, Velocity};
    use dawn_sector::ship_types::SHIP_TYPE_NPC_FRIGATE;

    const CELL_SIZE: f64 = 100.0;
    const PLAYER: PlayerId = PlayerId(1);

    #[derive(Debug, PartialEq, Eq)]
    enum Sent {
        Enter(u64),
        Leave(u64),
        Events(usize),
        Redirect,
    }

    struct FakeSession {
        player_id: PlayerId,
        ship_id: ShipId,
        sent: Vec<Sent>,
    }

    impl FakeSession {
        fn new(ship_id: ShipId) -> Self {
            Self {
                player_id: PLAYER,
                ship_id,
                sent: Vec::new(),
            }
        }
    }

    impl RuntimeAoiSession for FakeSession {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn ship_id(&self) -> ShipId {
            self.ship_id
        }

        fn send_redirect(&mut self, _ws_addr: SocketAddr) {
            self.sent.push(Sent::Redirect);
        }

        fn deliver<S: EventStore>(
            &mut self,
            frame: &mut AoiFrame,
            node: &SimulationNode<S>,
            new_events: &[DomainEvent],
            warp_arrivals: &[ShipId],
        ) -> bool {
            let observer = Observer {
                player_id: self.player_id,
                ship_id: self.ship_id,
            };
            frame.deliver_observer(self, node, observer, new_events, warp_arrivals)
        }
    }

    impl AoiSink for FakeSession {
        fn send_events(&mut self, events: &[DomainEvent]) -> bool {
            self.sent.push(Sent::Events(events.len()));
            true
        }

        fn send_message(&mut self, message: &ServerMessage) -> bool {
            match message {
                ServerMessage::AoiEnter(ship) => self.sent.push(Sent::Enter(ship.ship_id)),
                ServerMessage::AoiLeave { ship_id } => self.sent.push(Sent::Leave(*ship_id)),
                _ => {}
            }
            true
        }
    }

    fn initial_node() -> (SimulationNode, ShipId, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(7),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        );
        let own = node.spawn_ship(SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let leaving = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        (node, own, leaving)
    }

    fn next_node() -> (SimulationNode, ShipId, ShipId, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(7),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        );
        let own = node.spawn_ship(SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let leaving = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let entering = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        (node, own, leaving, entering)
    }

    #[test]
    fn production_admission_rebuilds_recovery_state_and_uses_the_runtime_adapter() {
        let (initial, own, leaving) = initial_node();
        let (next, next_own, next_leaving, entering) = next_node();
        assert_eq!(own, next_own);
        assert_eq!(leaving, next_leaving);

        let mut frame = AoiFrame::new(CELL_SIZE);
        let mut sessions = vec![FakeSession::new(own)];
        seed_runtime_session(&mut frame, &initial, &sessions[0]);

        deliver_runtime_sessions(
            &mut frame,
            &initial,
            &mut sessions,
            &HashMap::new(),
            &[],
            &[],
            &HashMap::new(),
        );
        assert_eq!(sessions[0].sent, vec![Sent::Events(0)]);
        sessions[0].sent.clear();

        deliver_runtime_sessions(
            &mut frame,
            &next,
            &mut sessions,
            &HashMap::new(),
            &[],
            &[],
            &HashMap::new(),
        );
        assert_eq!(
            sessions[0].sent,
            vec![
                Sent::Enter(entering.raw()),
                Sent::Leave(leaving.raw()),
                Sent::Events(0),
            ]
        );
    }
}
