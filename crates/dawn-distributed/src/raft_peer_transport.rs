//! Raft adapter over the shared [`crate::peer_transport`] lifecycle.

use crate::peer_transport::{PeerMessageKind, PeerTransport};
use crate::{actor::RaftActorMessage, rpc::RaftMessage, transport::RaftTransport};
use dawn_core::NodeId;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Serialize, Deserialize)]
struct PeerRaftEnvelope {
    sender: NodeId,
    message: RaftMessage,
}

/// Adapts shared peer frames to the Raft actor mailbox.
#[derive(Debug, Clone)]
pub struct PeerRaftTransport {
    peer: PeerTransport,
}

impl PeerRaftTransport {
    /// Start forwarding only Raft control frames from `peer` into `inbox`.
    pub fn from_peer_transport(
        peer: PeerTransport,
        inbox: UnboundedSender<RaftActorMessage>,
    ) -> Self {
        let mut frames = peer.subscribe();
        tokio::spawn(async move {
            while let Ok(frame) = frames.recv().await {
                if frame.kind != PeerMessageKind::RaftControl {
                    continue;
                }
                let Ok(envelope) = postcard::from_bytes::<PeerRaftEnvelope>(&frame.payload) else {
                    continue;
                };
                if envelope.sender != frame.from.node_id {
                    continue;
                }
                if inbox
                    .send(RaftActorMessage::Raft(envelope.message))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { peer }
    }
}

impl RaftTransport for PeerRaftTransport {
    fn send(&self, to: NodeId, msg: RaftMessage) {
        let Ok(payload) = postcard::to_stdvec(&PeerRaftEnvelope {
            sender: self.peer.local_identity().node_id,
            message: msg,
        }) else {
            return;
        };
        let _ = self.peer.send(to, PeerMessageKind::RaftControl, payload);
    }
}
