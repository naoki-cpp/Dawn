//! Client admission adapter for production Sector nodes.
//!
//! `main.rs` owns process wiring. This module owns socket waiting and session
//! promotion, while `dawn-sector::client_admission` owns the authoritative
//! begin/commit/abort lifecycle and every rollback decision.

use dawn_actor::ws_server;
use dawn_core::{Position, SectorId};
use dawn_event_store::store::EventStore;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal, CommittedClientAdmission,
};
use dawn_sector::node::SimulationNode;
use std::sync::Arc;
use tokio::sync::mpsc;

type HandshakeCompletion = (
    ClientAdmissionAttempt,
    Result<ws_server::PlayerSession, String>,
);

pub(crate) struct ClientAdmission {
    handshake_req_rx: mpsc::UnboundedReceiver<ws_server::HandshakeRequest>,
    completion_tx: mpsc::UnboundedSender<HandshakeCompletion>,
    completion_rx: mpsc::UnboundedReceiver<HandshakeCompletion>,
    ready_sess_tx: mpsc::UnboundedSender<ws_server::PlayerSession>,
    ready_sess_rx: mpsc::UnboundedReceiver<ws_server::PlayerSession>,
}

impl ClientAdmission {
    pub(crate) fn start(server: Arc<ws_server::WsServer>) -> Self {
        let (handshake_req_tx, handshake_req_rx) =
            mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
        let (completion_tx, completion_rx) = mpsc::unbounded_channel::<HandshakeCompletion>();
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
            completion_tx,
            completion_rx,
            ready_sess_tx,
            ready_sess_rx,
        }
    }

    pub(crate) fn advance_handshakes<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        sector_id: SectorId,
        aoi_cell_size: f64,
    ) {
        // Socket tasks report only their outcome. The tick-loop thread resolves
        // the Sector-owned attempt so authoritative mutation stays single-owner.
        while let Ok((attempt, result)) = self.completion_rx.try_recv() {
            if let Some((session, _committed)) = finish_admission(node, attempt, result) {
                let _ = self.ready_sess_tx.send(session);
            }
        }

        while let Ok(request) = self.handshake_req_rx.try_recv() {
            let intent = match request.resume {
                Some(resume) => ClientAdmissionIntent::Resume {
                    player_id: resume.player_id,
                    ship_id: resume.ship_id,
                },
                None => ClientAdmissionIntent::Fresh {
                    spawn_position: Position::new(30_000.0, 0.0, 0.0),
                },
            };

            let mut attempt = match node.begin_client_admission(intent, aoi_cell_size) {
                Ok(attempt) => attempt,
                Err(ClientAdmissionRefusal::ResumeShipMissing { ship_id, .. }) => {
                    eprintln!(
                        "[Node] resume refused from {}: ship #{} is not in Sector {}",
                        request.peer_addr,
                        ship_id.raw(),
                        sector_id.0
                    );
                    continue;
                }
                Err(ClientAdmissionRefusal::FreshAtPopulationCap) => {
                    eprintln!(
                        "[Node] connection from {} refused: at population cap",
                        request.peer_addr
                    );
                    continue;
                }
                Err(ClientAdmissionRefusal::MissingObserver(error)) => {
                    eprintln!(
                        "[Node] handshake refused from {}: {error}",
                        request.peer_addr
                    );
                    continue;
                }
            };

            if attempt.is_resumed() {
                println!(
                    "[Node] resume attempt from {}: {:?} ship #{}",
                    request.peer_addr,
                    attempt.player_id(),
                    attempt.ship_id().raw()
                );
            }

            let player_id = attempt.player_id();
            let ship_id = attempt.ship_id();
            let payload = attempt.take_handoff_payload();
            let completion_tx = self.completion_tx.clone();

            tokio::spawn(async move {
                let result = request
                    .complete(
                        player_id,
                        ship_id,
                        payload.initial_state,
                        payload.player_loadout,
                    )
                    .await
                    .map_err(|error| error.to_string());
                let _ = completion_tx.send((attempt, result));
            });
        }
    }

    pub(crate) fn try_recv_ready_session(&mut self) -> Option<ws_server::PlayerSession> {
        self.ready_sess_rx.try_recv().ok()
    }
}

fn finish_admission<S: EventStore, T>(
    node: &mut SimulationNode<S>,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match result {
        Ok(value) => match attempt.commit(node) {
            Ok(committed) => Some((value, committed)),
            Err(error) => {
                eprintln!("[Node] {error}");
                None
            }
        },
        Err(error) => {
            attempt.abort(node);
            eprintln!("[Node] handshake failed: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, SectorBounds, ShipTypeId, Velocity};

    const AOI_CELL_SIZE: f64 = 1_000.0;

    fn test_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn production_adapter_commits_successful_fresh_attempt() {
        let mut node = test_node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");

        assert_eq!(
            finish_admission(&mut node, attempt, Ok::<_, String>(())).map(|(value, _)| value),
            Some(())
        );
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn production_adapter_aborts_fresh_attempt_after_async_disconnect() {
        let mut node = test_node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");

        assert_eq!(
            finish_admission::<_, ()>(
                &mut node,
                attempt,
                Err("client disconnected while sending InitialState".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn production_adapter_failed_resume_keeps_pre_existing_ship() {
        let mut node = test_node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");

        assert_eq!(
            finish_admission::<_, ()>(&mut node, attempt, Err("client disconnected".to_string()),),
            None
        );
        assert_eq!(node.ship_count(), 1);
        assert!(node.ship_absolute_pos(ship_id).is_some());
        assert!(!node.apply_stop_command_owned(player_id, ship_id));
    }

    #[test]
    fn advance_handshakes_drains_failed_async_completion_and_aborts() {
        let mut node = test_node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        assert_eq!(node.ship_count(), 0);

        let (_request_tx, handshake_req_rx) =
            mpsc::unbounded_channel::<ws_server::HandshakeRequest>();
        let (completion_tx, completion_rx) = mpsc::unbounded_channel::<HandshakeCompletion>();
        let (ready_sess_tx, ready_sess_rx) = mpsc::unbounded_channel::<ws_server::PlayerSession>();
        completion_tx
            .send((attempt, Err("client disconnected".to_string())))
            .expect("completion receiver alive");
        let mut admission = ClientAdmission {
            handshake_req_rx,
            completion_tx,
            completion_rx,
            ready_sess_tx,
            ready_sess_rx,
        };

        admission.advance_handshakes(&mut node, SectorId(3), AOI_CELL_SIZE);

        assert_eq!(node.ship_count(), 0);
    }
}
