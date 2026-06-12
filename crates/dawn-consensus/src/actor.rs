//! `RaftActor` — wraps [`RaftState`] in a tokio task driven entirely by its
//! mailbox (FBD-004: no direct method calls between actors).
//!
//! The actor receives three kinds of messages:
//! - [`RaftActorMessage::Raft`]: an incoming RPC from a peer (via `RaftTransport`)
//! - [`RaftActorMessage::TickElapsed`]: the Tick loop's Step 10 (ADR-0014 §7)
//! - [`RaftActorMessage::GetRole`]: a query for tests/observability

use crate::rpc::RaftMessage;
use crate::state::{RaftState, Role, TickEffect};
use crate::transport::RaftTransport;
use dawn_core::NodeId;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Messages delivered to a `RaftActor`'s mailbox.
#[derive(Debug)]
pub enum RaftActorMessage {
    /// An RPC received from a peer's transport.
    Raft(RaftMessage),

    /// One logical Tick elapsed (ADR-0014 §7 Step 10).
    TickElapsed,

    /// Query the current role and term. For tests/observability only.
    GetRole(oneshot::Sender<(Role, crate::state::Term)>),

    /// Stop the actor's run loop.
    Shutdown,
}

/// The Raft actor: owns a [`RaftState`] and a [`RaftTransport`] for sending
/// RPCs to peers. All interaction happens via `RaftActorMessage` (FBD-004).
pub struct RaftActor {
    state    : RaftState,
    peers    : Vec<NodeId>,
    transport: Arc<dyn RaftTransport>,
    rx       : mpsc::UnboundedReceiver<RaftActorMessage>,
}

impl RaftActor {
    pub fn new(
        state: RaftState,
        peers: Vec<NodeId>,
        transport: Arc<dyn RaftTransport>,
        rx: mpsc::UnboundedReceiver<RaftActorMessage>,
    ) -> Self {
        Self { state, peers, transport, rx }
    }

    /// Run the actor's mailbox loop until [`RaftActorMessage::Shutdown`] is
    /// received or the mailbox is closed.
    pub async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                RaftActorMessage::Raft(raft_msg) => self.handle_raft_message(raft_msg),
                RaftActorMessage::TickElapsed => self.handle_tick(),
                RaftActorMessage::GetRole(reply) => {
                    let _ = reply.send((self.state.role.clone(), self.state.current_term));
                }
                RaftActorMessage::Shutdown => break,
            }
        }
    }

    fn handle_tick(&mut self) {
        for effect in self.state.on_tick() {
            match effect {
                TickEffect::StartElection { term } => {
                    let req = RaftMessage::RequestVote(crate::rpc::RequestVote {
                        term,
                        candidate_id: self.state.node_id,
                    });
                    self.broadcast(req);
                }
                TickEffect::SendHeartbeat { term } => {
                    let req = RaftMessage::AppendEntries(crate::rpc::AppendEntries {
                        term,
                        leader_id: self.state.node_id,
                    });
                    self.broadcast(req);
                }
            }
        }
    }

    fn handle_raft_message(&mut self, msg: RaftMessage) {
        match msg {
            RaftMessage::RequestVote(req) => {
                let candidate_id = req.candidate_id;
                let resp = self.state.handle_request_vote(&req);
                self.transport.send(candidate_id, RaftMessage::RequestVoteResponse(resp));
            }
            RaftMessage::RequestVoteResponse(resp) => {
                let became_leader = self.state.record_vote(resp.voter, resp.term);
                if became_leader {
                    // Announce leadership immediately rather than waiting for
                    // the next heartbeat tick, so followers recognize the new
                    // leader without delay.
                    let term = self.state.current_term;
                    let req = RaftMessage::AppendEntries(crate::rpc::AppendEntries {
                        term,
                        leader_id: self.state.node_id,
                    });
                    self.broadcast(req);
                }
            }
            RaftMessage::AppendEntries(req) => {
                let leader_id = req.leader_id;
                let resp = self.state.handle_append_entries(&req);
                self.transport.send(leader_id, RaftMessage::AppendEntriesResponse(resp));
            }
            RaftMessage::AppendEntriesResponse(_) => {
                // No log replication yet; nothing to do with the response.
            }
        }
    }

    fn broadcast(&self, msg: RaftMessage) {
        for &peer in &self.peers {
            self.transport.send(peer, msg.clone());
        }
    }
}

/// Cloneable handle for sending messages to a running [`RaftActor`].
#[derive(Clone)]
pub struct RaftActorHandle {
    tx: mpsc::UnboundedSender<RaftActorMessage>,
}

impl RaftActorHandle {
    pub fn new(tx: mpsc::UnboundedSender<RaftActorMessage>) -> Self {
        Self { tx }
    }

    /// Send `TickElapsed` (ADR-0014 §7 Step 10). Best-effort: ignored if the
    /// actor has shut down.
    pub fn tick(&self) {
        let _ = self.tx.send(RaftActorMessage::TickElapsed);
    }

    pub async fn role(&self) -> (Role, crate::state::Term) {
        let (tx, rx) = oneshot::channel();
        self.tx.send(RaftActorMessage::GetRole(tx)).expect("RaftActor is no longer running");
        rx.await.expect("RaftActor dropped reply sender")
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(RaftActorMessage::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RaftState;
    use crate::transport::{InProcessTransport, PartitionableTransport};
    use std::collections::HashMap;

    fn node(id: u8) -> NodeId {
        NodeId(id)
    }

    #[tokio::test]
    async fn lone_node_with_no_peers_starts_election_after_timeout() {
        let (tx, rx) = mpsc::unbounded_channel();
        let transport = Arc::new(InProcessTransport::new(HashMap::new()));
        let state = RaftState::new(node(0), vec![], 3, 1);
        let actor = RaftActor::new(state, vec![], transport, rx);
        tokio::spawn(actor.run());

        let handle = RaftActorHandle::new(tx);
        for _ in 0..3 {
            handle.tick();
        }

        // Give the actor a moment to process the queued ticks.
        tokio::task::yield_now().await;
        let (role, term) = handle.role().await;
        // With no peers there is no one to grant a vote, so the node
        // remains Candidate (it cannot reach a majority by itself).
        assert_eq!(role, Role::Candidate);
        assert_eq!(term, crate::state::Term(1));

        handle.shutdown();
    }

    #[tokio::test]
    async fn three_node_cluster_elects_a_single_leader() {
        let ids = [node(0), node(1), node(2)];
        let mut txs = Vec::new();
        let mut rxs = Vec::new();
        for _ in &ids {
            let (tx, rx) = mpsc::unbounded_channel();
            txs.push(tx);
            rxs.push(Some(rx));
        }

        let partitioned = PartitionableTransport::new_partition_set();
        let mut handles = Vec::new();

        for (i, &id) in ids.iter().enumerate() {
            let peers: Vec<NodeId> = ids.iter().copied().filter(|&p| p != id).collect();
            let mut peer_txs = HashMap::new();
            for (j, &peer_id) in ids.iter().enumerate() {
                if peer_id != id {
                    peer_txs.insert(peer_id, txs[j].clone());
                }
            }
            let transport: Arc<dyn RaftTransport> = Arc::new(PartitionableTransport::new(
                id,
                InProcessTransport::new(peer_txs),
                partitioned.clone(),
            ));

            // Node 0 has the shortest election timeout so it becomes
            // Candidate first; the others have long enough timeouts that
            // they won't also start an election before Node 0's votes land.
            let election_timeout = if i == 0 { 3 } else { 50 };
            let state = RaftState::new(id, peers.clone(), election_timeout, 2);

            let rx = rxs[i].take().unwrap();
            let actor = RaftActor::new(state, peers, transport, rx);
            tokio::spawn(actor.run());
            handles.push(RaftActorHandle::new(txs[i].clone()));
        }

        // Advance Node 0 past its election timeout. Its RequestVote messages
        // are delivered to peers' mailboxes; yielding lets all three actors
        // process the resulting message exchange.
        for _ in 0..3 {
            for handle in &handles {
                handle.tick();
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        let mut leaders = 0;
        let mut terms = std::collections::HashSet::new();
        for handle in &handles {
            let (role, term) = handle.role().await;
            terms.insert(term);
            if role == Role::Leader {
                leaders += 1;
            }
        }

        assert_eq!(leaders, 1, "exactly one node should be leader");
        assert_eq!(terms.len(), 1, "all nodes should agree on the term");

        for handle in &handles {
            handle.shutdown();
        }
    }
}
