//! In-process transport for `RaftActor`s (ADR-0014 §6).
//!
//! Phase 7 runs all nodes in a single process, so `RaftMessage`s are
//! delivered via `mpsc` channels rather than a real network. Node failure
//! is simulated by [`PartitionableTransport`], which silently drops
//! messages to/from partitioned nodes.

use crate::actor::RaftActorMessage;
use crate::rpc::RaftMessage;
use dawn_core::NodeId;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// Delivers [`RaftMessage`]s between `RaftActor`s.
///
/// `send` is synchronous and best-effort: if the destination mailbox is full
/// or closed, the message is silently dropped (matches real network UDP-like
/// semantics — Raft is designed to tolerate message loss).
pub trait RaftTransport: Send + Sync {
    fn send(&self, to: NodeId, msg: RaftMessage);
}

/// Routes [`RaftMessage`]s to peer mailboxes within the same process.
pub struct InProcessTransport {
    peers: HashMap<NodeId, UnboundedSender<RaftActorMessage>>,
}

impl InProcessTransport {
    pub fn new(peers: HashMap<NodeId, UnboundedSender<RaftActorMessage>>) -> Self {
        Self { peers }
    }
}

impl RaftTransport for InProcessTransport {
    fn send(&self, to: NodeId, msg: RaftMessage) {
        if let Some(tx) = self.peers.get(&to) {
            // Best-effort: a closed receiver means the peer has shut down.
            let _ = tx.send(RaftActorMessage::Raft(msg));
        }
    }
}

/// Wraps an [`InProcessTransport`] with a shared, mutable set of partitioned
/// nodes for fault injection in tests.
///
/// A message is dropped if either the sender or the recipient is currently
/// partitioned. The partition set is shared (via `Arc<Mutex<_>>`) across all
/// nodes' transports so tests can call [`Self::partition`] / [`Self::heal`]
/// from a single handle and affect the whole cluster.
pub struct PartitionableTransport {
    inner      : InProcessTransport,
    self_id    : NodeId,
    partitioned: Arc<Mutex<HashSet<NodeId>>>,
}

impl PartitionableTransport {
    pub fn new(
        self_id: NodeId,
        inner: InProcessTransport,
        partitioned: Arc<Mutex<HashSet<NodeId>>>,
    ) -> Self {
        Self { inner, self_id, partitioned }
    }

    /// Create a fresh, empty partition set shared by all nodes in a cluster.
    pub fn new_partition_set() -> Arc<Mutex<HashSet<NodeId>>> {
        Arc::new(Mutex::new(HashSet::new()))
    }

    /// Cut `node` off from the rest of the cluster: messages to and from it
    /// are dropped by every `PartitionableTransport` sharing this set.
    pub fn partition(partitioned: &Arc<Mutex<HashSet<NodeId>>>, node: NodeId) {
        partitioned.lock().unwrap().insert(node);
    }

    /// Restore `node`'s connectivity.
    pub fn heal(partitioned: &Arc<Mutex<HashSet<NodeId>>>, node: NodeId) {
        partitioned.lock().unwrap().remove(&node);
    }
}

impl RaftTransport for PartitionableTransport {
    fn send(&self, to: NodeId, msg: RaftMessage) {
        let partitioned = self.partitioned.lock().unwrap();
        if partitioned.contains(&self.self_id) || partitioned.contains(&to) {
            return;
        }
        drop(partitioned);
        self.inner.send(to, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{AppendEntries, RequestVote};
    use crate::state::Term;
    use tokio::sync::mpsc;

    fn node(id: u8) -> NodeId {
        NodeId(id)
    }

    #[test]
    fn in_process_transport_delivers_message_to_target_mailbox() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut peers = HashMap::new();
        peers.insert(node(1), tx);
        let transport = InProcessTransport::new(peers);

        let msg = RaftMessage::RequestVote(RequestVote { term: Term(1), candidate_id: node(0) });
        transport.send(node(1), msg.clone());

        let received = rx.try_recv().unwrap();
        assert!(matches!(received, RaftActorMessage::Raft(m) if m == msg));
    }

    #[test]
    fn in_process_transport_silently_drops_message_to_unknown_node() {
        let transport = InProcessTransport::new(HashMap::new());
        // Should not panic.
        transport.send(node(99), RaftMessage::RequestVote(RequestVote { term: Term(1), candidate_id: node(0) }));
    }

    #[test]
    fn partitioned_node_does_not_receive_messages() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut peers = HashMap::new();
        peers.insert(node(1), tx);
        let inner = InProcessTransport::new(peers);

        let partitioned = PartitionableTransport::new_partition_set();
        let transport = PartitionableTransport::new(node(0), inner, partitioned.clone());

        PartitionableTransport::partition(&partitioned, node(1));
        transport.send(node(1), RaftMessage::AppendEntries(AppendEntries::heartbeat(Term(1), node(0))));

        assert!(rx.try_recv().is_err(), "partitioned node should not receive message");
    }

    #[test]
    fn healed_node_receives_messages_again() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut peers = HashMap::new();
        peers.insert(node(1), tx);
        let inner = InProcessTransport::new(peers);

        let partitioned = PartitionableTransport::new_partition_set();
        let transport = PartitionableTransport::new(node(0), inner, partitioned.clone());

        PartitionableTransport::partition(&partitioned, node(1));
        PartitionableTransport::heal(&partitioned, node(1));

        let msg = RaftMessage::AppendEntries(AppendEntries::heartbeat(Term(1), node(0)));
        transport.send(node(1), msg.clone());

        let received = rx.try_recv().unwrap();
        assert!(matches!(received, RaftActorMessage::Raft(m) if m == msg));
    }

    #[test]
    fn partitioned_sender_cannot_send_messages() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut peers = HashMap::new();
        peers.insert(node(1), tx);
        let inner = InProcessTransport::new(peers);

        let partitioned = PartitionableTransport::new_partition_set();
        let transport = PartitionableTransport::new(node(0), inner, partitioned.clone());

        PartitionableTransport::partition(&partitioned, node(0));
        transport.send(node(1), RaftMessage::AppendEntries(AppendEntries::heartbeat(Term(1), node(0))));

        assert!(rx.try_recv().is_err(), "partitioned sender's messages should be dropped");
    }
}
