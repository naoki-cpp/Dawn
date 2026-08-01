//! Client admission for production Sector nodes.
//!
//! `main.rs` owns process wiring; this module owns the client admission state
//! machine: accept raw WebSocket sockets, read Hello, choose fresh vs. resume
//! identity, complete Welcome/InitialState, and surface ready sessions.

use dawn_actor::ws_server;
use dawn_core::{PlayerId, Position, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_sector::node::{HandoffPayload, MissingObserverShip, SimulationNode};
use dawn_wire::ResumeIdentity;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) struct ClientAdmission {
    handshake_req_rx: mpsc::UnboundedReceiver<ws_server::HandshakeRequest>,
    ready_sess_tx: mpsc::UnboundedSender<ws_server::PlayerSession>,
    ready_sess_rx: mpsc::UnboundedReceiver<ws_server::PlayerSession>,
    /// A fresh-spawn ship whose handshake completion failed (client
    /// disconnected mid-handshake) and must be despawned. Populated from
    /// inside the spawned completion task, which cannot hold `&mut
    /// SimulationNode` itself (its lifetime doesn't extend across the
    /// `.await`) -- `advance_handshakes` (which does own `node`) drains this
    /// each call. Never populated for a resumed ship: that ship existed
    /// before this attempt, so removing it would destroy state unrelated to
    /// the failure (see `despawn_incomplete_handshake_spawn`'s doc comment).
    failed_fresh_spawn_rx: mpsc::UnboundedReceiver<ShipId>,
    failed_fresh_spawn_tx: mpsc::UnboundedSender<ShipId>,
}

impl ClientAdmission {
    pub(crate) fn start(server: Arc<ws_server::WsServer>) -> Self {
        let (handshake_req_tx, handshake_req_rx) =
            mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
        let (ready_sess_tx, ready_sess_rx) = mpsc::unbounded_channel::<ws_server::PlayerSession>();
        let (failed_fresh_spawn_tx, failed_fresh_spawn_rx) = mpsc::unbounded_channel::<ShipId>();

        tokio::spawn(async move {
            loop {
                if let Some((stream, addr)) = server.try_accept_raw().await {
                    let tx = handshake_req_tx.clone();
                    tokio::spawn(async move {
                        match ws_server::WsServer::accept_handshake_request(stream, addr).await {
                            Ok(req) => {
                                let _ = tx.send(req);
                            }
                            Err(e) => eprintln!("[Node] handshake request failed: {e}"),
                        }
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        Self {
            handshake_req_rx,
            ready_sess_tx,
            ready_sess_rx,
            failed_fresh_spawn_rx,
            failed_fresh_spawn_tx,
        }
    }

    pub(crate) fn advance_handshakes<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        sector_id: SectorId,
        aoi_cell_size: f64,
    ) {
        while let Ok(ship_id) = self.failed_fresh_spawn_rx.try_recv() {
            node.despawn_incomplete_handshake_spawn(ship_id);
        }

        while let Ok(request) = self.handshake_req_rx.try_recv() {
            let handshake_identity = match select_handshake_identity(node, request.resume) {
                HandshakeSelection::Selected(identity) => identity,
                HandshakeSelection::RefusedResumeMissingShip(resume) => {
                    eprintln!(
                        "[Node] resume refused from {}: ship #{} is not in Sector {}",
                        request.peer_addr,
                        resume.ship_id.raw(),
                        sector_id.0
                    );
                    continue;
                }
                HandshakeSelection::RefusedFreshAtCap => {
                    eprintln!(
                        "[Node] connection from {} refused: at population cap",
                        request.peer_addr
                    );
                    continue;
                }
            };

            if handshake_identity.resumed {
                println!(
                    "[Node] resume accepted from {}: {:?} ship #{}",
                    request.peer_addr,
                    handshake_identity.player_id,
                    handshake_identity.ship_id.raw()
                );
            }

            let player_id = handshake_identity.player_id;
            let ship_id = handshake_identity.ship_id;
            let payload = match build_handshake_payload(node, &handshake_identity, aoi_cell_size) {
                Ok(payload) => payload,
                Err(error) => {
                    eprintln!(
                        "[Node] handshake refused from {}: {error}",
                        request.peer_addr
                    );
                    if let Some(ship_id) = fresh_spawn_for_failed_handshake(&handshake_identity) {
                        node.despawn_incomplete_handshake_spawn(ship_id);
                    }
                    continue;
                }
            };
            let tx = self.ready_sess_tx.clone();
            let despawn_on_failure = fresh_spawn_for_failed_handshake(&handshake_identity);
            let failed_fresh_spawn_tx = self.failed_fresh_spawn_tx.clone();

            tokio::spawn(async move {
                match request
                    .complete(
                        player_id,
                        ship_id,
                        payload.initial_state,
                        payload.player_loadout,
                    )
                    .await
                {
                    Ok(sess) => {
                        let _ = tx.send(sess);
                    }
                    Err(e) => {
                        eprintln!("[Node] handshake failed: {e}");
                        if let Some(ship_id) = despawn_on_failure {
                            let _ = failed_fresh_spawn_tx.send(ship_id);
                        }
                    }
                }
            });
        }
    }

    pub(crate) fn try_recv_ready_session(&mut self) -> Option<ws_server::PlayerSession> {
        self.ready_sess_rx.try_recv().ok()
    }
}

struct HandshakeIdentity {
    player_id: PlayerId,
    ship_id: ShipId,
    resumed: bool,
}

enum HandshakeSelection {
    Selected(HandshakeIdentity),
    RefusedFreshAtCap,
    RefusedResumeMissingShip(ResumeIdentity),
}

fn select_handshake_identity<S: EventStore>(
    node: &mut SimulationNode<S>,
    resume: Option<ResumeIdentity>,
) -> HandshakeSelection {
    if let Some(resume) = resume {
        return if node.resume_player_ship(resume.ship_id, resume.player_id) {
            HandshakeSelection::Selected(HandshakeIdentity {
                player_id: resume.player_id,
                ship_id: resume.ship_id,
                resumed: true,
            })
        } else {
            HandshakeSelection::RefusedResumeMissingShip(resume)
        };
    }

    if node.at_population_cap() {
        return HandshakeSelection::RefusedFreshAtCap;
    }

    let player_id = node.next_player_id();
    let ship_id = node.spawn_player_ship_at_pub(player_id, Position::new(30_000.0, 0.0, 0.0));
    HandshakeSelection::Selected(HandshakeIdentity {
        player_id,
        ship_id,
        resumed: false,
    })
}

fn build_handshake_payload<S: EventStore>(
    node: &SimulationNode<S>,
    identity: &HandshakeIdentity,
    aoi_cell_size: f64,
) -> Result<HandoffPayload, MissingObserverShip> {
    node.build_handoff_payload(identity.ship_id, aoi_cell_size)
}

/// Return the fresh-spawn ship that must be removed when a handshake cannot
/// complete. A resumed ship predates this attempt and must never be removed as
/// cleanup for an admission or transport failure (ADR-0007 §2-A resume).
fn fresh_spawn_for_failed_handshake(identity: &HandshakeIdentity) -> Option<ShipId> {
    (!identity.resumed).then_some(identity.ship_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds};

    fn test_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn fresh_handshake_spawns_new_player_ship() {
        let mut node = test_node();

        let selection = select_handshake_identity(&mut node, None);

        let HandshakeSelection::Selected(identity) = selection else {
            panic!("fresh handshake should be accepted below the population cap");
        };
        assert!(!identity.resumed);
        assert_eq!(identity.player_id, PlayerId(0));
        assert_eq!(node.ship_count(), 1);
        assert_eq!(
            node.ship_absolute_pos(identity.ship_id),
            Some(dawn_core::AbsolutePosition::new(30_000.0, 0.0, 0.0))
        );
    }

    #[test]
    fn resume_handshake_adopts_existing_ship_without_spawning() {
        let mut node = test_node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(42.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );

        let selection =
            select_handshake_identity(&mut node, Some(ResumeIdentity { player_id, ship_id }));

        let HandshakeSelection::Selected(identity) = selection else {
            panic!("resume handshake should adopt a ship already in this sector");
        };
        assert!(identity.resumed);
        assert_eq!(identity.player_id, player_id);
        assert_eq!(identity.ship_id, ship_id);
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn resume_handshake_rejects_missing_ship_without_spawning() {
        let mut node = test_node();
        let resume = ResumeIdentity {
            player_id: PlayerId(12),
            ship_id: ShipId::new(NodeId(99), 1),
        };

        let selection = select_handshake_identity(&mut node, Some(resume));

        let HandshakeSelection::RefusedResumeMissingShip(rejected) = selection else {
            panic!("resume handshake should reject a ship absent from this sector");
        };
        assert_eq!(rejected, resume);
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn fresh_handshake_payload_rejects_a_missing_observer() {
        let node = test_node();
        let identity = HandshakeIdentity {
            player_id: PlayerId(1),
            ship_id: ShipId::new(NodeId(7), 999),
            resumed: false,
        };

        let error = build_handshake_payload(&node, &identity, 1_000.0)
            .expect_err("fresh identity without an observer must be refused");

        assert_eq!(error.ship_id, identity.ship_id);
    }

    #[test]
    fn resumed_handshake_payload_rejects_a_missing_observer() {
        let node = test_node();
        let identity = HandshakeIdentity {
            player_id: PlayerId(12),
            ship_id: ShipId::new(NodeId(7), 999),
            resumed: true,
        };

        let error = build_handshake_payload(&node, &identity, 1_000.0)
            .expect_err("resume identity without an observer must be refused");

        assert_eq!(error.ship_id, identity.ship_id);
    }

    #[test]
    fn fresh_handshake_respects_population_cap() {
        let mut node = test_node();
        node.set_population_cap(0);

        let selection = select_handshake_identity(&mut node, None);

        assert!(matches!(selection, HandshakeSelection::RefusedFreshAtCap));
        assert_eq!(node.ship_count(), 0);
    }

    // -- Completion-failure cleanup (ghost ship on a dropped connection) -----

    #[test]
    fn a_fresh_spawn_is_marked_for_despawn_on_completion_failure() {
        let identity = HandshakeIdentity {
            player_id: PlayerId(0),
            ship_id: ShipId::new(NodeId(0), 1),
            resumed: false,
        };
        assert_eq!(
            fresh_spawn_for_failed_handshake(&identity),
            Some(identity.ship_id)
        );
    }

    #[test]
    fn a_resumed_ship_is_never_marked_for_despawn_on_completion_failure() {
        let identity = HandshakeIdentity {
            player_id: PlayerId(0),
            ship_id: ShipId::new(NodeId(0), 1),
            resumed: true,
        };
        assert_eq!(fresh_spawn_for_failed_handshake(&identity), None);
    }

    #[test]
    fn advance_handshakes_despawns_a_ship_reported_as_a_failed_fresh_spawn() {
        let mut node = test_node();
        let ship_id = node.spawn_player_ship_at_pub(PlayerId(0), Position::ORIGIN);
        assert_eq!(node.ship_count(), 1);

        let (_req_tx, handshake_req_rx) = mpsc::unbounded_channel();
        let (ready_sess_tx, ready_sess_rx) = mpsc::unbounded_channel();
        let (failed_fresh_spawn_tx, failed_fresh_spawn_rx) = mpsc::unbounded_channel();
        failed_fresh_spawn_tx.send(ship_id).unwrap();
        let mut admission = ClientAdmission {
            handshake_req_rx,
            ready_sess_tx,
            ready_sess_rx,
            failed_fresh_spawn_rx,
            failed_fresh_spawn_tx,
        };

        admission.advance_handshakes(&mut node, SectorId(3), 1_000.0);

        assert_eq!(
            node.ship_count(),
            0,
            "a ship reported as a failed fresh spawn must be despawned"
        );
    }
}
