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

    /// Submit a proposal to the Raft Log (ADR-0014 §3 \[1\]).
    ///
    /// If this node is the Leader it appends the entry; otherwise it
    /// forwards the payload to the leader it currently recognizes. The
    /// proposal is silently dropped if no leader is known yet — callers
    /// must treat proposals as at-most-once until committed.
    Propose(Vec<u8>),

    /// Query the current role and term. For tests/observability only.
    GetRole(oneshot::Sender<(Role, crate::state::Term)>),

    /// Stop the actor's run loop.
    Shutdown,
}

/// The Raft actor: owns a [`RaftState`] and a [`RaftTransport`] for sending
/// RPCs to peers. All interaction happens via `RaftActorMessage` (FBD-004).
pub struct RaftActor {
    state: RaftState,
    peers: Vec<NodeId>,
    transport: Arc<dyn RaftTransport>,
    rx: mpsc::UnboundedReceiver<RaftActorMessage>,
    /// Committed entry payloads are delivered here in log order, exactly
    /// once each, for the owning node to apply (ADR-0014 §7 Step 7.5).
    committed_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Number of committed entries already delivered to `committed_tx`.
    last_delivered: u64,
}

impl RaftActor {
    pub fn new(
        state: RaftState,
        peers: Vec<NodeId>,
        transport: Arc<dyn RaftTransport>,
        rx: mpsc::UnboundedReceiver<RaftActorMessage>,
        committed_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Self {
        Self {
            state,
            peers,
            transport,
            rx,
            committed_tx,
            last_delivered: 0,
        }
    }

    /// Run the actor's mailbox loop until [`RaftActorMessage::Shutdown`] is
    /// received or the mailbox is closed.
    pub async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            let role_before = self.state.role.clone();
            match msg {
                RaftActorMessage::Raft(raft_msg) => self.handle_raft_message(raft_msg),
                RaftActorMessage::TickElapsed => self.handle_tick(),
                RaftActorMessage::Propose(payload) => self.handle_propose(payload),
                RaftActorMessage::GetRole(reply) => {
                    let _ = reply.send((self.state.role.clone(), self.state.current_term));
                }
                RaftActorMessage::Shutdown => break,
            }
            // Surfaces every role transition (election won/lost, step-down on
            // higher term) for 8D-5 field observability — these are the
            // events most likely to reveal where physical-node latency or
            // packet loss breaks the SimulationNode/Actor boundary (P9-1).
            if self.state.role != role_before {
                eprintln!(
                    "[Raft node={:?}] {:?} → {:?} (term={:?})",
                    self.state.node_id, role_before, self.state.role, self.state.current_term
                );
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
                TickEffect::SendHeartbeat { .. } => {
                    self.replicate_to_all();
                }
            }
        }
    }

    /// Append a proposal as Leader, or forward it to the recognized Leader.
    fn handle_propose(&mut self, payload: Vec<u8>) {
        if self.state.propose(payload.clone()) {
            self.deliver_committed(); // a lone node commits immediately
            self.replicate_to_all();
        } else if let Some(leader) = self.state.leader_id {
            self.transport
                .send(leader, RaftMessage::ProposeForward { payload });
        }
        // No known leader: drop. The caller re-issues the command.
    }

    fn handle_raft_message(&mut self, msg: RaftMessage) {
        match msg {
            RaftMessage::RequestVote(req) => {
                let candidate_id = req.candidate_id;
                let resp = self.state.handle_request_vote(&req);
                self.transport
                    .send(candidate_id, RaftMessage::RequestVoteResponse(resp));
            }
            RaftMessage::RequestVoteResponse(resp) => {
                let became_leader = self.state.record_vote(resp.voter, resp.term);
                if became_leader {
                    // Announce leadership immediately rather than waiting for
                    // the next heartbeat tick, so followers recognize the new
                    // leader without delay.
                    self.replicate_to_all();
                }
            }
            RaftMessage::AppendEntries(req) => {
                let leader_id = req.leader_id;
                let resp = self.state.handle_append_entries(&req);
                self.deliver_committed();
                self.transport
                    .send(leader_id, RaftMessage::AppendEntriesResponse(resp));
            }
            RaftMessage::AppendEntriesResponse(resp) => {
                if resp.term > self.state.current_term {
                    self.state.become_follower(resp.term);
                    return;
                }
                if resp.success {
                    let advanced = self
                        .state
                        .record_replication(resp.responder, resp.match_index);
                    if advanced {
                        self.deliver_committed();
                        // Propagate the new commit index without waiting for
                        // the next heartbeat tick.
                        self.replicate_to_all();
                    }
                } else {
                    self.state
                        .record_rejection(resp.responder, resp.match_index);
                }
            }
            RaftMessage::ProposeForward { payload } => {
                // A non-leader node forwarded a proposal to us.
                self.handle_propose(payload);
            }
        }
    }

    /// As Leader, send each peer the log suffix it is missing (or an empty
    /// heartbeat when it is up to date), with the current commit index.
    fn replicate_to_all(&mut self) {
        if self.state.role != Role::Leader {
            return;
        }
        let term = self.state.current_term;
        let leader_commit = self.state.commit_index();
        for &peer in &self.peers {
            let (prev_log_index, prev_log_term, entries) = self.state.entries_for(peer);
            self.transport.send(
                peer,
                RaftMessage::AppendEntries(crate::rpc::AppendEntries {
                    term,
                    leader_id: self.state.node_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                }),
            );
        }
    }

    /// Deliver newly committed payloads to the owning node, in log order.
    fn deliver_committed(&mut self) {
        let committed: Vec<Vec<u8>> = self
            .state
            .committed_from(self.last_delivered)
            .iter()
            .map(|e| e.payload.clone())
            .collect();
        self.last_delivered += committed.len() as u64;
        for payload in committed {
            // Best-effort: the receiver may have shut down in tests.
            let _ = self.committed_tx.send(payload);
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

    /// Submit a proposal to the Raft Log (ADR-0014 §3). Best-effort: the
    /// proposal is dropped if no leader is known yet; it only takes effect
    /// once committed and delivered via the committed-entries channel.
    pub fn propose(&self, payload: Vec<u8>) {
        let _ = self.tx.send(RaftActorMessage::Propose(payload));
    }

    pub async fn role(&self) -> (Role, crate::state::Term) {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(RaftActorMessage::GetRole(tx))
            .expect("RaftActor is no longer running");
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
    async fn lone_node_with_no_peers_becomes_leader_after_timeout() {
        let (tx, rx) = mpsc::unbounded_channel();
        let transport = Arc::new(InProcessTransport::new(HashMap::new()));
        let state = RaftState::new(node(0), vec![], 3, 1);
        let (committed_tx, _committed_rx) = mpsc::unbounded_channel();
        let actor = RaftActor::new(state, vec![], transport, rx, committed_tx);
        tokio::spawn(actor.run());

        let handle = RaftActorHandle::new(tx);
        for _ in 0..3 {
            handle.tick();
        }

        // Give the actor a moment to process the queued ticks.
        tokio::task::yield_now().await;
        let (role, term) = handle.role().await;
        // A single-node cluster's self-vote is already a majority.
        assert_eq!(role, Role::Leader);
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
            let (committed_tx, _committed_rx) = mpsc::unbounded_channel();
            let actor = RaftActor::new(state, peers, transport, rx, committed_tx);
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

    /// Spawn a 3-node actor cluster where node 0 wins the first election.
    /// Returns the handles and each node's committed-entries receiver.
    fn spawn_three_node_cluster() -> (Vec<RaftActorHandle>, Vec<mpsc::UnboundedReceiver<Vec<u8>>>) {
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
        let mut committed_rxs = Vec::new();

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

            let election_timeout = if i == 0 { 3 } else { 50 };
            let state = RaftState::new(id, peers.clone(), election_timeout, 2);

            let rx = rxs[i].take().unwrap();
            let (committed_tx, committed_rx) = mpsc::unbounded_channel();
            tokio::spawn(RaftActor::new(state, peers, transport, rx, committed_tx).run());
            handles.push(RaftActorHandle::new(txs[i].clone()));
            committed_rxs.push(committed_rx);
        }

        (handles, committed_rxs)
    }

    async fn settle(handles: &[RaftActorHandle], ticks: usize) {
        for _ in 0..ticks {
            for handle in handles {
                handle.tick();
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn committed_proposal_is_delivered_to_every_node_exactly_once() {
        let (handles, mut committed_rxs) = spawn_three_node_cluster();
        settle(&handles, 4).await;
        assert_eq!(handles[0].role().await.0, Role::Leader);

        handles[0].propose(b"transit-1".to_vec());
        // Replication, majority ACK, and commit propagation all ride on
        // ticks (heartbeats) and immediate-response paths.
        settle(&handles, 6).await;

        for (i, rx) in committed_rxs.iter_mut().enumerate() {
            let payload = rx
                .try_recv()
                .unwrap_or_else(|_| panic!("node {i} did not receive the committed entry"));
            assert_eq!(payload, b"transit-1", "node {i}");
            assert!(
                rx.try_recv().is_err(),
                "node {i} received the entry more than once"
            );
        }
    }

    #[tokio::test]
    async fn proposal_submitted_to_a_follower_is_forwarded_to_the_leader_and_commits() {
        let (handles, mut committed_rxs) = spawn_three_node_cluster();
        settle(&handles, 4).await;
        assert_eq!(handles[1].role().await.0, Role::Follower);

        // Submit via a follower: it forwards to the leader it learned
        // from AppendEntries.
        handles[1].propose(b"via-follower".to_vec());
        settle(&handles, 6).await;

        for (i, rx) in committed_rxs.iter_mut().enumerate() {
            let payload = rx
                .try_recv()
                .unwrap_or_else(|_| panic!("node {i} did not receive the committed entry"));
            assert_eq!(payload, b"via-follower", "node {i}");
        }
    }
}
