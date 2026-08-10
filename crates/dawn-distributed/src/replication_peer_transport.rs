//! Replication and catch-up adapters over the shared peer transport.

use crate::peer_transport::{PeerFrame, PeerMessageKind, PeerTransport};
use crate::{
    CatchUpMessage, CatchUpPayload, CatchUpResponse, CatchUpTransport, LogBatch, ReplicaSnapshot,
    ReplicationTransport,
};
use dawn_core::{NodeId, SectorId};
use dawn_storage::{DurabilityTransportMessage, JournalError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 10_000;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DurabilitySendError {
    #[error("failed to encode durability message: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("failed to send durability message: {0}")]
    Peer(#[from] crate::peer_transport::PeerSendError),
    #[error("invalid durability message: {0}")]
    Journal(#[from] JournalError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotFrameHeader {
    request_id: u64,
    requester_sector_id: SectorId,
    owner_sector_id: SectorId,
    snapshot_sector_id: SectorId,
    snapshot_log_index: u64,
    owner_next_index: u64,
    snapshot_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CatchUpRangeFrameHeader {
    request_id: u64,
    requester_sector_id: SectorId,
    owner_sector_id: SectorId,
    owner_next_index: u64,
}

/// Adapts shared control/bulk frames to the replication and catch-up traits.
///
/// Ordinary batches and catch-up control messages use the shared control
/// channel. Snapshot bytes use the shared bulk channel and are kept outside
/// the postcard envelope; only their small routing header is serialized.
#[derive(Debug, Clone)]
pub struct PeerReplicationTransport {
    peer: PeerTransport,
    inbound_batch_tx: broadcast::Sender<LogBatch>,
    inbound_catch_up_tx: broadcast::Sender<CatchUpMessage>,
    inbound_durability_tx: broadcast::Sender<DurabilityTransportMessage>,
    inbound_repository_tx: broadcast::Sender<PeerFrame>,
}

impl PeerReplicationTransport {
    pub fn from_peer_transport(peer: PeerTransport) -> Self {
        let (inbound_batch_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (inbound_catch_up_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (inbound_durability_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (inbound_repository_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let mut frames = peer.subscribe();
        let batch_tx = inbound_batch_tx.clone();
        let catch_up_tx = inbound_catch_up_tx.clone();
        let durability_tx = inbound_durability_tx.clone();
        let repository_tx = inbound_repository_tx.clone();
        let local_sector_id = peer.local_identity().sector_id;
        tokio::spawn(async move {
            while let Ok(frame) = frames.recv().await {
                match frame.kind {
                    PeerMessageKind::RecoveryRange => {
                        if frame.metadata.is_empty() {
                            let Ok(batch) = postcard::from_bytes::<LogBatch>(&frame.payload) else {
                                continue;
                            };
                            if batch.sector_id == frame.from.sector_id {
                                let _ = batch_tx.send(batch);
                            }
                        } else if let Ok(header) =
                            postcard::from_bytes::<CatchUpRangeFrameHeader>(&frame.metadata)
                        {
                            let Ok(batch) = postcard::from_bytes::<LogBatch>(&frame.payload) else {
                                continue;
                            };
                            if header.requester_sector_id == local_sector_id
                                && header.owner_sector_id == frame.from.sector_id
                                && batch.sector_id == header.owner_sector_id
                            {
                                let _ =
                                    catch_up_tx.send(CatchUpMessage::Response(CatchUpResponse {
                                        request_id: header.request_id,
                                        requester_sector_id: header.requester_sector_id,
                                        owner_sector_id: header.owner_sector_id,
                                        payload: CatchUpPayload::Suffix {
                                            batch,
                                            owner_next_index: header.owner_next_index,
                                        },
                                    }));
                            }
                        }
                    }
                    PeerMessageKind::CatchUpControl => {
                        if let Ok(message) = postcard::from_bytes::<CatchUpMessage>(&frame.payload)
                        {
                            let sender_sector = frame.from.sector_id;
                            let sender_matches = match &message {
                                CatchUpMessage::Request(request) => {
                                    request.requester_sector_id == sender_sector
                                }
                                CatchUpMessage::Response(response) => {
                                    response.owner_sector_id == sender_sector
                                }
                            };
                            if sender_matches {
                                let _ = catch_up_tx.send(message);
                            }
                        }
                    }
                    PeerMessageKind::DurabilityControl => {
                        if let Ok(message) =
                            postcard::from_bytes::<DurabilityTransportMessage>(&frame.payload)
                        {
                            let sector_matches = match &message {
                                DurabilityTransportMessage::Stage { context, .. }
                                | DurabilityTransportMessage::Receipt { context, .. } => {
                                    context.sector_id == frame.from.sector_id
                                }
                            };
                            if sector_matches && message.validate().is_ok() {
                                let _ = durability_tx.send(message);
                            }
                        }
                    }
                    PeerMessageKind::Snapshot => {
                        if let Ok(header) =
                            postcard::from_bytes::<SnapshotFrameHeader>(&frame.metadata)
                        {
                            if header.requester_sector_id == local_sector_id
                                && header.owner_sector_id == frame.from.sector_id
                                && header.snapshot_sector_id == header.owner_sector_id
                                && header.snapshot_len as usize == frame.payload.len()
                            {
                                let _ =
                                    catch_up_tx.send(CatchUpMessage::Response(CatchUpResponse {
                                        request_id: header.request_id,
                                        requester_sector_id: header.requester_sector_id,
                                        owner_sector_id: header.owner_sector_id,
                                        payload: CatchUpPayload::Snapshot {
                                            snapshot: ReplicaSnapshot::new(
                                                header.snapshot_sector_id,
                                                header.snapshot_log_index,
                                                frame.payload,
                                            ),
                                            owner_next_index: header.owner_next_index,
                                        },
                                    }));
                            }
                        }
                    }
                    PeerMessageKind::RepositoryCatchUp => {
                        let _ = repository_tx.send(frame);
                    }
                    PeerMessageKind::RaftControl => {}
                }
            }
        });
        Self {
            peer,
            inbound_batch_tx,
            inbound_catch_up_tx,
            inbound_durability_tx,
            inbound_repository_tx,
        }
    }

    fn send_to_sector(&self, sector_id: SectorId, kind: PeerMessageKind, payload: Vec<u8>) {
        if let Some(peer) = self.peer.peer_for_sector(sector_id) {
            let _ = self.peer.send(peer, kind, payload);
        }
    }

    fn send_snapshot(&self, response: CatchUpResponse, snapshot: ReplicaSnapshot) {
        let Some(peer) = self.peer.peer_for_sector(response.requester_sector_id) else {
            return;
        };
        let CatchUpPayload::Snapshot {
            owner_next_index, ..
        } = response.payload
        else {
            return;
        };
        let Ok(snapshot_len) = u32::try_from(snapshot.bytes.len()) else {
            return;
        };
        let header = SnapshotFrameHeader {
            request_id: response.request_id,
            requester_sector_id: response.requester_sector_id,
            owner_sector_id: response.owner_sector_id,
            snapshot_sector_id: snapshot.sector_id,
            snapshot_log_index: snapshot.log_index,
            owner_next_index,
            snapshot_len,
        };
        let Ok(metadata) = postcard::to_stdvec(&header) else {
            return;
        };
        let _ = self.peer.send_split(
            peer,
            PeerMessageKind::Snapshot,
            metadata,
            snapshot.bytes.as_ref().clone(),
        );
    }
}

impl ReplicationTransport for PeerReplicationTransport {
    fn broadcast(&self, batch: LogBatch) {
        let Ok(payload) = postcard::to_stdvec(&batch) else {
            return;
        };
        for peer in self.peer.peer_identities() {
            let _ = self.peer.send(
                peer.node_id,
                PeerMessageKind::RecoveryRange,
                payload.clone(),
            );
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<LogBatch> {
        self.inbound_batch_tx.subscribe()
    }
}

impl CatchUpTransport for PeerReplicationTransport {
    fn send_catch_up(&self, message: CatchUpMessage) {
        match message {
            CatchUpMessage::Request(request) => {
                if let Ok(payload) = postcard::to_stdvec(&CatchUpMessage::Request(request.clone()))
                {
                    self.send_to_sector(
                        request.owner_sector_id,
                        PeerMessageKind::CatchUpControl,
                        payload,
                    );
                }
            }
            CatchUpMessage::Response(response) => match response.payload.clone() {
                CatchUpPayload::Suffix {
                    batch,
                    owner_next_index,
                } => {
                    let Some(peer) = self.peer.peer_for_sector(response.requester_sector_id) else {
                        return;
                    };
                    let header = CatchUpRangeFrameHeader {
                        request_id: response.request_id,
                        requester_sector_id: response.requester_sector_id,
                        owner_sector_id: response.owner_sector_id,
                        owner_next_index,
                    };
                    let Ok(metadata) = postcard::to_stdvec(&header) else {
                        return;
                    };
                    let Ok(payload) = postcard::to_stdvec(&batch) else {
                        return;
                    };
                    let _ = self.peer.send_split(
                        peer,
                        PeerMessageKind::RecoveryRange,
                        metadata,
                        payload,
                    );
                }
                CatchUpPayload::Snapshot { snapshot, .. } => self.send_snapshot(response, snapshot),
                _ => {
                    if let Ok(payload) =
                        postcard::to_stdvec(&CatchUpMessage::Response(response.clone()))
                    {
                        self.send_to_sector(
                            response.requester_sector_id,
                            PeerMessageKind::CatchUpControl,
                            payload,
                        );
                    }
                }
            },
        }
    }

    fn subscribe_catch_up(&self) -> broadcast::Receiver<CatchUpMessage> {
        self.inbound_catch_up_tx.subscribe()
    }
}

/// Retained bytes can be transferred through the same bulk channel without
/// making the transport responsible for their repository schema.
impl PeerReplicationTransport {
    pub fn send_repository_catch_up(
        &self,
        peer: NodeId,
        metadata: Vec<u8>,
        payload: Vec<u8>,
    ) -> Result<(), crate::peer_transport::PeerSendError> {
        self.peer
            .send_split(peer, PeerMessageKind::RepositoryCatchUp, metadata, payload)
    }

    pub fn send_durability(
        &self,
        peer: NodeId,
        message: DurabilityTransportMessage,
    ) -> Result<(), DurabilitySendError> {
        message.validate()?;
        let payload = postcard::to_stdvec(&message)?;
        self.peer
            .send(peer, PeerMessageKind::DurabilityControl, payload)?;
        Ok(())
    }

    pub fn subscribe_durability(&self) -> broadcast::Receiver<DurabilityTransportMessage> {
        self.inbound_durability_tx.subscribe()
    }

    /// Receive repository-owned metadata and bytes without coupling this
    /// crate to the #277 SQLite schema.
    pub fn subscribe_repository_catch_up(&self) -> broadcast::Receiver<PeerFrame> {
        self.inbound_repository_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_transport::{
        PeerCapabilities, PeerEndpoint, PeerIdentity, PeerTransportConfig,
    };
    use crate::CatchUpRequest;
    use dawn_core::NodeId;
    use tokio::time::{sleep, timeout, Duration};

    fn identity(node: u8, sector: u8) -> PeerIdentity {
        PeerIdentity {
            node_id: NodeId(node),
            sector_id: SectorId(sector),
        }
    }

    async fn pair() -> (PeerReplicationTransport, PeerReplicationTransport) {
        let b = PeerTransport::bind(PeerTransportConfig {
            local_identity: identity(2, 2),
            control_addr: "127.0.0.1:0".parse().unwrap(),
            bulk_addr: "127.0.0.1:0".parse().unwrap(),
            capabilities: PeerCapabilities::RECOVERY_CATCH_UP,
            peers: vec![PeerEndpoint {
                identity: identity(1, 1),
                control_addr: "127.0.0.1:9".parse().unwrap(),
                bulk_addr: "127.0.0.1:10".parse().unwrap(),
            }],
        })
        .await
        .unwrap();
        let a = PeerTransport::bind(PeerTransportConfig {
            local_identity: identity(1, 1),
            control_addr: "127.0.0.1:0".parse().unwrap(),
            bulk_addr: "127.0.0.1:0".parse().unwrap(),
            capabilities: PeerCapabilities::RECOVERY_CATCH_UP,
            peers: vec![PeerEndpoint {
                identity: identity(2, 2),
                control_addr: b.control_addr(),
                bulk_addr: b.bulk_addr(),
            }],
        })
        .await
        .unwrap();
        (
            PeerReplicationTransport::from_peer_transport(a),
            PeerReplicationTransport::from_peer_transport(b),
        )
    }

    #[tokio::test]
    async fn adapter_routes_recovery_ranges_and_bulk_snapshots() {
        let (a, b) = pair().await;
        let mut batch_rx = b.subscribe();
        let mut catch_up_rx = b.subscribe_catch_up();
        let mut reverse_catch_up_rx = a.subscribe_catch_up();
        sleep(Duration::from_millis(50)).await;

        a.broadcast(LogBatch::new(SectorId(1), 4, Vec::new()));
        let batch = timeout(Duration::from_secs(2), batch_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(batch.sector_id, SectorId(1));
        assert_eq!(batch.from_index, 4);

        a.send_catch_up(CatchUpMessage::Request(CatchUpRequest {
            request_id: 7,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(2),
            from_index: 4,
            max_events: 32,
        }));
        assert!(matches!(
            timeout(Duration::from_secs(2), catch_up_rx.recv())
                .await
                .unwrap()
                .unwrap(),
            CatchUpMessage::Request(CatchUpRequest { request_id: 7, .. })
        ));

        b.send_catch_up(CatchUpMessage::Response(CatchUpResponse {
            request_id: 9,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(2),
            payload: CatchUpPayload::Suffix {
                batch: LogBatch::new(SectorId(2), 4, Vec::new()),
                owner_next_index: 16,
            },
        }));
        let suffix = timeout(Duration::from_secs(2), reverse_catch_up_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            suffix,
            CatchUpMessage::Response(CatchUpResponse {
                request_id: 9,
                payload: CatchUpPayload::Suffix { .. },
                ..
            })
        ));

        b.send_catch_up(CatchUpMessage::Response(CatchUpResponse {
            request_id: 8,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(2),
            payload: CatchUpPayload::Snapshot {
                snapshot: ReplicaSnapshot::new(SectorId(2), 12, vec![5; 1024]),
                owner_next_index: 16,
            },
        }));
        let snapshot = timeout(Duration::from_secs(2), reverse_catch_up_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            snapshot,
            CatchUpMessage::Response(CatchUpResponse {
                request_id: 8,
                payload: CatchUpPayload::Snapshot { .. },
                ..
            })
        ));
    }
}
