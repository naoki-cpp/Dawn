//! Client admission for production Sector nodes.
//!
//! `main.rs` owns process wiring; this module owns the client admission state
//! machine: accept raw WebSocket sockets, read Hello, choose fresh vs. resume
//! identity, complete Welcome/InitialState, and surface ready sessions.

use dawn_actor::{protocol::ResumeIdentity, ws_server};
use dawn_core::{PlayerId, Position, SectorId, ShipId};
use dawn_event_store::store::EventStore;
use dawn_sector::node::SimulationNode;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) struct ClientAdmission {
    handshake_req_rx: mpsc::UnboundedReceiver<ws_server::HandshakeRequest>,
    ready_sess_tx: mpsc::UnboundedSender<ws_server::PlayerSession>,
    ready_sess_rx: mpsc::UnboundedReceiver<ws_server::PlayerSession>,
}

impl ClientAdmission {
    pub(crate) fn start(server: Arc<ws_server::WsServer>) -> Self {
        let (handshake_req_tx, handshake_req_rx) =
            mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
        let (ready_sess_tx, ready_sess_rx) = mpsc::unbounded_channel::<ws_server::PlayerSession>();

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
        }
    }

    pub(crate) fn advance_handshakes<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        sector_id: SectorId,
        aoi_cell_size: f32,
    ) {
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
            let initial_state = node
                .ship_absolute_pos(ship_id)
                .map(|pos| node.build_initial_state_json_for(pos, aoi_cell_size))
                .unwrap_or_else(|| node.build_initial_state_json());
            let player_fitting = node.build_player_fitting_json(ship_id);
            let tx = self.ready_sess_tx.clone();

            tokio::spawn(async move {
                match request
                    .complete(player_id, ship_id, &initial_state, player_fitting)
                    .await
                {
                    Ok(sess) => {
                        let _ = tx.send(sess);
                    }
                    Err(e) => eprintln!("[Node] handshake failed: {e}"),
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
        return if node.adopt_player_ship(resume.ship_id, resume.player_id) {
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
            Some([30_000.0, 0.0, 0.0])
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
    fn fresh_handshake_respects_population_cap() {
        let mut node = test_node();
        node.set_population_cap(0);

        let selection = select_handshake_identity(&mut node, None);

        assert!(matches!(selection, HandshakeSelection::RefusedFreshAtCap));
        assert_eq!(node.ship_count(), 0);
    }
}
