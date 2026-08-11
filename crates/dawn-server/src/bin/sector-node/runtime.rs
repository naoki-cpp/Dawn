//! Production Sector Node runtime frame orchestration.
//!
//! `main.rs` owns process wiring: config, TCP transports, and async accept
//! channels. This module owns one production Node frame: command dispatch,
//! jump proposal fallback, runtime tick stepping, outbound replication,
//! Redirect handling, and AoI delivery.

use crate::runtime_frame::{OwnedRaftRuntimeConsensus, RuntimeFrameHost};
use dawn_actor::ws_server;
use dawn_core::{DomainEvent, SectorId, ShipId};
use dawn_distributed::{OutboundLogPublisher, PeerReplicationTransport};
use dawn_protocol::ServerMessage;
use dawn_sector::aoi::{AoiMessage, AoiSink, Observer};
use dawn_sector::aoi_frame::{deliver_sector_sessions, AoiFrame, AoiSessionCallbacks};
use dawn_sector::node::{
    ClientRequestAdmissionError, JumpOutcome, RuntimeCommandDispatch, SimulationNode,
};
use dawn_storage::{EventStore, FileJournal};
use std::collections::HashMap;
use std::net::SocketAddr;

fn client_request_rejection(
    error: ClientRequestAdmissionError,
) -> dawn_protocol::ClientRequestRejectionWire {
    match error {
        ClientRequestAdmissionError::Validation(error) => {
            dawn_protocol::ClientRequestRejectionWire::validation(error)
        }
        ClientRequestAdmissionError::NoActiveShip => {
            dawn_protocol::ClientRequestRejectionWire::no_active_ship()
        }
        ClientRequestAdmissionError::UnsupportedRequest { request } => {
            dawn_protocol::ClientRequestRejectionWire::unsupported_request(request)
        }
    }
}

pub(crate) struct SectorNodeRuntime {
    host: RuntimeFrameHost<FileJournal, OwnedRaftRuntimeConsensus>,
    sector_id: SectorId,
    peer_ws: HashMap<SectorId, SocketAddr>,
    sessions: Vec<ws_server::PlayerSession>,
    aoi_frame: AoiFrame,
    outbound_replication: OutboundLogPublisher<PeerReplicationTransport>,
}

impl SectorNodeRuntime {
    pub(crate) fn new(
        host: RuntimeFrameHost<FileJournal, OwnedRaftRuntimeConsensus>,
        sector_id: SectorId,
        aoi_cell_size: f64,
        peer_ws: HashMap<SectorId, SocketAddr>,
        repl_transport: PeerReplicationTransport,
        event_store: &impl EventStore,
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
            host,
        }
    }

    pub(crate) fn with_node_mut<R>(
        &mut self,
        operation: impl FnOnce(&mut SimulationNode) -> R,
    ) -> R {
        self.host.with_node_mut(operation)
    }

    pub(crate) fn with_state_mut<R>(
        &mut self,
        operation: impl FnOnce(&mut SimulationNode, &mut FileJournal) -> R,
    ) -> R {
        self.host.with_state_mut(operation)
    }

    pub(crate) fn promote_ready_session(&mut self, sess: ws_server::PlayerSession) {
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
        self.aoi_frame.seed_observer(
            self.host.node(),
            Observer {
                player_id: sess.player_id,
                ship_id: sess.ship_id,
            },
        );
        self.sessions.push(sess);
    }

    pub(crate) fn run_frame(&mut self, event_store: &mut impl EventStore) -> anyhow::Result<()> {
        let (lock_commands, pending_jumps) = self.collect_player_commands();
        self.propose_player_jumps(pending_jumps);

        let outbound_replication = &mut self.outbound_replication;
        let output = self
            .host
            .run_frame_with_output(&lock_commands, |node, _, events| {
                event_store.append_batch(events.to_vec());
                outbound_replication.publish_events(node.sector_id(), events);
            })
            .map_err(|error| anyhow::anyhow!("authoritative recovery tick failed: {error}"))?;

        self.log_auto_jumps(&output.pending_auto_jumps);
        let jumped_ships = self.jumped_ships(&output.events);
        self.deliver_frames(&output.events, &output.completed_warps, &jumped_ships);
        Ok(())
    }

    fn collect_player_commands(
        &mut self,
    ) -> (
        Vec<dawn_core::LockOnCommand>,
        Vec<(usize, ShipId, dawn_core::JumpCommand)>,
    ) {
        let mut lock_commands = Vec::new();
        let mut pending_jumps = Vec::new();

        for dispatch in self.host.collect_runtime_commands(
            &mut self.sessions,
            &mut lock_commands,
            |session| session.player_id,
            ws_server::PlayerSession::try_recv_request,
        ) {
            match dispatch {
                RuntimeCommandDispatch::Jump {
                    session_index,
                    ship_id,
                    command,
                } => pending_jumps.push((session_index, ship_id, command)),
                RuntimeCommandDispatch::RefreshPlayerLoadout {
                    session_index,
                    player_id,
                } => {
                    if let Some(loadout) = self
                        .host
                        .node()
                        .build_player_loadout_json_for_player(player_id)
                    {
                        if let Some(session) = self.sessions.get_mut(session_index) {
                            session.send_message(&ServerMessage::PlayerLoadout(loadout));
                        }
                    }
                }
                RuntimeCommandDispatch::Rejected {
                    session_index,
                    error,
                } => {
                    if let Some(session) = self.sessions.get_mut(session_index) {
                        session.send_message(&ServerMessage::ClientRequestRejected(
                            client_request_rejection(error),
                        ));
                    }
                }
            }
        }

        (lock_commands, pending_jumps)
    }

    fn propose_player_jumps(
        &mut self,
        pending_jumps: Vec<(usize, ShipId, dawn_core::JumpCommand)>,
    ) {
        for (idx, ship_id, j) in pending_jumps {
            let Some(sess) = self.sessions.get(idx) else {
                continue;
            };
            if ship_id != sess.ship_id {
                continue;
            }
            let outcome = self.host.propose_jump(ship_id, j.gate_id);
            match outcome {
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

    fn deliver_frames(
        &mut self,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        deliver_sector_sessions(
            &mut self.aoi_frame,
            self.host.node(),
            &mut self.sessions,
            new_events,
            warp_arrivals,
            jumped_ships,
            AoiSessionCallbacks {
                player_id: |session: &ws_server::PlayerSession| session.player_id,
                ship_id: |session: &ws_server::PlayerSession| session.ship_id,
                deliver: |session: &mut ws_server::PlayerSession,
                          frame: &mut AoiFrame,
                          node: &SimulationNode,
                          new_events: &[DomainEvent],
                          warp_arrivals: &[ShipId]| {
                    let observer = Observer {
                        player_id: session.player_id,
                        ship_id: session.ship_id,
                    };
                    let mut sink = SessionSink(session);
                    frame.deliver_observer(&mut sink, node, observer, new_events, warp_arrivals)
                },
                on_redirect: |session: &mut ws_server::PlayerSession, destination| {
                    if let Some(&ws_addr) = self.peer_ws.get(&destination) {
                        send_redirect(session, ws_addr);
                        println!("[Node] Redirect {:?} -> {ws_addr}", session.player_id);
                    }
                },
            },
        );
    }
}

fn send_redirect(session: &mut ws_server::PlayerSession, ws_addr: SocketAddr) {
    session.conn.send_message(&ServerMessage::Redirect {
        ws_addr: ws_addr.to_string(),
        resume_ticket: session.resume_ticket,
    });
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// the trait lives in dawn-sector, the type in dawn-actor, so the impl has to
/// live here where both are foreign).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_aoi_message(&mut self, msg: &AoiMessage) -> bool {
        let message = msg.to_server_message();
        self.0.send_message(&message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, Velocity};
    use dawn_sector::ship_types::SHIP_TYPE_NPC_FRIGATE;

    const CELL_SIZE: f64 = 100.0;
    const PLAYER: PlayerId = PlayerId(1);

    fn test_catalog() -> std::sync::Arc<dawn_sector::game_data::GameDataCatalog> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::sync::Arc::new(
            dawn_sector::game_data::GameDataCatalog::load_from_paths(
                root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
                root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
            )
            .expect("repository game-data catalog"),
        )
    }

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

    impl AoiSink for FakeSession {
        fn send_aoi_message(&mut self, message: &AoiMessage) -> bool {
            match message {
                AoiMessage::AoiEnter(ship) => self.sent.push(Sent::Enter(ship.ship_id)),
                AoiMessage::AoiLeave { ship_id } => self.sent.push(Sent::Leave(*ship_id)),
                AoiMessage::Fact(_) => self.sent.push(Sent::Events(1)),
                AoiMessage::MotionCorrection { .. } | AoiMessage::PositionSnap { .. } => {}
            }
            true
        }
    }

    fn deliver_fake_session<V: dawn_sector::view::SectorView>(
        session: &mut FakeSession,
        frame: &mut AoiFrame,
        node: &V,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let observer = Observer {
            player_id: session.player_id,
            ship_id: session.ship_id,
        };
        frame.deliver_observer(session, node, observer, new_events, warp_arrivals)
    }

    fn initial_node() -> (SimulationNode, ShipId, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(7),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            test_catalog(),
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
            test_catalog(),
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
        frame.seed_observer(
            &initial,
            Observer {
                player_id: sessions[0].player_id,
                ship_id: sessions[0].ship_id,
            },
        );

        deliver_sector_sessions(
            &mut frame,
            &initial,
            &mut sessions,
            &[],
            &[],
            &HashMap::new(),
            AoiSessionCallbacks {
                player_id: |session: &FakeSession| session.player_id,
                ship_id: |session: &FakeSession| session.ship_id,
                deliver: deliver_fake_session,
                on_redirect: |session: &mut FakeSession, _| session.sent.push(Sent::Redirect),
            },
        );
        assert!(sessions[0].sent.is_empty());
        sessions[0].sent.clear();

        deliver_sector_sessions(
            &mut frame,
            &next,
            &mut sessions,
            &[],
            &[],
            &HashMap::new(),
            AoiSessionCallbacks {
                player_id: |session: &FakeSession| session.player_id,
                ship_id: |session: &FakeSession| session.ship_id,
                deliver: deliver_fake_session,
                on_redirect: |session: &mut FakeSession, _| session.sent.push(Sent::Redirect),
            },
        );
        assert_eq!(
            sessions[0].sent,
            vec![Sent::Enter(entering.raw()), Sent::Leave(leaving.raw()),]
        );
    }
}
