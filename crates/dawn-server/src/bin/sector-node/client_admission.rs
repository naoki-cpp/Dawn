//! Client admission adapter for production Sector nodes.
//!
//! `main.rs` owns process wiring. This module owns socket waiting and session
//! promotion, while `dawn-sector::client_admission` owns the authoritative
//! begin/commit/abort lifecycle and every rollback decision.

use dawn_core::SectorId;
use dawn_sector::client_admission::{
    ClientAdmissionAttempt, ClientAdmissionIntent, ClientAdmissionRefusal, CommittedClientAdmission,
};
#[cfg(test)]
use dawn_sector::client_admission_resolution::resolve_client_admission;
use dawn_sector::client_admission_resolution::ClientAdmissionResolution;
#[cfg(test)]
use dawn_sector::node::SimulationNode;
use dawn_server::runtime_frame::{RuntimeClientAdmissionError, RuntimeClientAdmissionHost};
use dawn_server::ws_server;
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

    pub(crate) fn advance_handshakes<H: RuntimeClientAdmissionHost>(
        &mut self,
        host: &mut H,
        sector_id: SectorId,
        aoi_cell_size: f64,
    ) {
        // Socket tasks report only their outcome. The tick-loop thread resolves
        // the Sector-owned attempt so authoritative mutation stays single-owner.
        while let Ok((attempt, result)) = self.completion_rx.try_recv() {
            if let Some((session, _committed)) = finish_admission(host, attempt, result) {
                let _ = self.ready_sess_tx.send(session);
            }
        }

        while let Ok(request) = self.handshake_req_rx.try_recv() {
            let intent = match request.resume {
                Some(resume) => ClientAdmissionIntent::Resume {
                    resume_ticket: resume,
                },
                None => ClientAdmissionIntent::Fresh {
                    spawn_position: host.default_player_spawn_position(),
                },
            };

            let mut attempt = match host.begin_client_admission(intent, aoi_cell_size) {
                Ok(attempt) => attempt,
                Err(RuntimeClientAdmissionError::Refused(
                    ClientAdmissionRefusal::ResumeTicketInvalid,
                )) => {
                    eprintln!(
                        "[Node] resume refused from {}: invalid resume ticket for Sector {}",
                        request.peer_addr, sector_id.0
                    );
                    continue;
                }
                Err(RuntimeClientAdmissionError::Refused(
                    ClientAdmissionRefusal::ResumeAlreadyPending { ship_id, .. },
                )) => {
                    eprintln!(
                        "[Node] resume refused from {}: ship #{} already has an in-flight resume",
                        request.peer_addr,
                        ship_id.raw()
                    );
                    continue;
                }
                Err(RuntimeClientAdmissionError::Refused(
                    ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. },
                )) => {
                    eprintln!(
                        "[Node] resume refused from {}: ship #{} conflicts with established ownership",
                        request.peer_addr,
                        ship_id.raw()
                    );
                    continue;
                }
                Err(RuntimeClientAdmissionError::Refused(
                    ClientAdmissionRefusal::FreshAtPopulationCap,
                )) => {
                    eprintln!(
                        "[Node] connection from {} refused: at population cap",
                        request.peer_addr
                    );
                    continue;
                }
                Err(RuntimeClientAdmissionError::Refused(
                    ClientAdmissionRefusal::MissingObserver(error),
                )) => {
                    eprintln!(
                        "[Node] handshake refused from {}: {error}",
                        request.peer_addr
                    );
                    continue;
                }
                Err(RuntimeClientAdmissionError::Host(error)) => {
                    eprintln!(
                        "[Node] handshake unavailable from {}: {error}",
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
            let resume_ticket = attempt.resume_ticket();
            let payload = attempt.take_handoff_payload();
            let completion_tx = self.completion_tx.clone();

            tokio::spawn(async move {
                let result = request
                    .complete(
                        player_id,
                        ship_id,
                        resume_ticket,
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

fn finish_admission<T, H: RuntimeClientAdmissionHost>(
    host: &mut H,
    attempt: ClientAdmissionAttempt,
    result: Result<T, String>,
) -> Option<(T, CommittedClientAdmission)> {
    match host.resolve_client_admission(attempt, result) {
        Ok(ClientAdmissionResolution::Committed { value, admission }) => Some((value, admission)),
        Ok(ClientAdmissionResolution::Aborted { error }) => {
            eprintln!("[Node] handshake failed: {error}");
            None
        }
        Ok(ClientAdmissionResolution::CommitRejected { error }) => {
            eprintln!("[Node] {error}");
            None
        }
        Err(error) => {
            eprintln!("[Node] admission host unavailable: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;
    use dawn_core::Position;
    use dawn_core::SectorBounds;

    const AOI_CELL_SIZE: f64 = 1_000.0;

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

    fn test_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            test_catalog(),
        )
    }

    fn commit_runtime_frame(node: &mut SimulationNode) {
        let mut journal = dawn_storage::InMemoryJournal::new();
        let mut consensus = dawn_sector::transit::LocalRuntimeConsensus;
        let mut health = dawn_sector::transit::RuntimeHealth::default();
        let transition_id = dawn_sector::transit::runtime_transition_id(node);
        dawn_sector::transit::run_durable_runtime_frame(
            node,
            &mut journal,
            &mut consensus,
            &dawn_sector::transit::LocalRuntimeDurabilityPolicy,
            &mut health,
            dawn_sector::transition::FrameInput::lock_only(&[]),
            dawn_sector::transit::DurableRuntimeTickContext {
                transition_id,
                owner_epoch: 0,
                durability: dawn_storage::DurabilityMode::Synced,
                profile: dawn_sector::transit::RuntimeDurabilityProfile::LocalDurable,
            },
            dawn_sector::transit::reconcile_runtime_repositories,
            |_, _, _| {},
        )
        .expect("admission frame should commit");
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
            finish_admission(
                &mut crate::TestAdmissionHost(&mut node),
                attempt,
                Ok::<_, String>(()),
            )
            .map(|(value, _)| value),
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
            finish_admission::<(), _>(
                &mut crate::TestAdmissionHost(&mut node),
                attempt,
                Err("client disconnected while sending InitialState".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[tokio::test]
    async fn production_adapter_aborts_after_real_socket_disconnect() {
        use futures_util::SinkExt;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
            socket
                .send(Message::Binary(
                    dawn_protocol::ClientMessage::Hello(dawn_protocol::HelloMessage {
                        resume: None,
                    })
                    .encode()
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            socket.get_mut().shutdown().await.unwrap();
        });
        let (stream, peer_addr) = listener.accept().await.unwrap();
        let request = ws_server::WsServer::accept_handshake_request(stream, peer_addr)
            .await
            .unwrap();
        client.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut node = test_node();
        let mut attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let player_id = attempt.player_id();
        let ship_id = attempt.ship_id();
        let payload = attempt.take_handoff_payload();
        let result = request
            .complete(
                player_id,
                ship_id,
                attempt.resume_ticket(),
                payload.initial_state,
                payload.player_loadout,
            )
            .await
            .map_err(|error| error.to_string());
        assert!(
            result.is_err(),
            "closed socket must fail the awaited handoff"
        );
        assert!(
            finish_admission(&mut crate::TestAdmissionHost(&mut node), attempt, result).is_none()
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn production_adapter_failed_resume_keeps_pre_existing_ship() {
        let mut node = test_node();
        let fresh = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let resume_ticket = fresh.resume_ticket();
        let committed = match resolve_client_admission(&mut node, fresh, Ok::<_, ()>(())) {
            ClientAdmissionResolution::Committed { admission, .. } => admission,
            other => panic!("fresh admission should commit, got {other:?}"),
        };
        let player_id = committed.player_id;
        let ship_id = committed.ship_id;
        commit_runtime_frame(&mut node);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("resume attempt");

        assert_eq!(
            finish_admission::<(), _>(
                &mut crate::TestAdmissionHost(&mut node),
                attempt,
                Err("client disconnected".to_string()),
            ),
            None
        );
        assert_eq!(node.ship_count(), 1);
        assert!(node.ship_absolute_pos(ship_id).is_some());
        assert!(node.apply_stop_command_owned(player_id, ship_id));
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

        admission.advance_handshakes(
            &mut crate::TestAdmissionHost(&mut node),
            SectorId(3),
            AOI_CELL_SIZE,
        );

        assert_eq!(node.ship_count(), 0);
    }
}
