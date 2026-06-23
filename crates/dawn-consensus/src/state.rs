//! Raft state machine: Follower / Candidate / Leader transitions.
//!
//! Timers (election timeout, heartbeat interval) are driven by logical Tick
//! counts via [`RaftState::on_tick`], never by physical time (INV-005,
//! FBD-003, ADR-0014 §5).
//!
//! This module covers only the state machine and election-timeout-driven
//! role transitions. RequestVote/AppendEntries message handling lives in
//! `rpc.rs`.

use dawn_core::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Raft term number. Monotonically increasing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize,
)]
pub struct Term(pub u64);

impl Term {
    pub const ZERO: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// One entry in the Raft Log (ADR-0014 §3).
///
/// The payload is an opaque byte string: `dawn-consensus` knows nothing
/// about domain types. Callers (dawn-simulation) serialize their own
/// proposal type (e.g. a Transit proposal) into it. The Raft Log holds
/// Commands (proposals), never Events (INV-006) — committed proposals are
/// turned into Events by the caller and appended to its EventStore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub term: Term,
    pub payload: Vec<u8>,
}

/// The role a node currently plays in the Raft cluster.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

/// Side effects produced by [`RaftState::on_tick`] that the caller (the
/// `RaftActor`) must act on, e.g. by sending RPCs via `RaftTransport`.
#[derive(Debug, Clone, PartialEq)]
pub enum TickEffect {
    /// Election timeout elapsed: the node became a Candidate and should
    /// broadcast RequestVote to all peers for the new term.
    StartElection { term: Term },

    /// As Leader, the heartbeat interval elapsed: broadcast AppendEntries
    /// (heartbeat) to all peers.
    SendHeartbeat { term: Term },
}

/// The Raft state machine for a single node.
///
/// Election timeout and heartbeat interval are expressed in logical Tick
/// counts (ADR-0014 §5). `on_tick` must be called once per Tick from
/// `SimulationNode`'s Step 10.
pub struct RaftState {
    pub node_id: NodeId,
    peers: Vec<NodeId>,

    pub role: Role,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,

    election_elapsed: u64,
    election_timeout: u64,

    heartbeat_elapsed: u64,
    heartbeat_interval: u64,

    /// Votes received in the current term (Candidate only). Includes self.
    votes_received: HashSet<NodeId>,

    /// The Raft Log (ADR-0014 §3). Append-only except for the standard Raft
    /// conflict-truncation on followers, which only ever removes
    /// *uncommitted* entries.
    pub(crate) log: Vec<LogEntry>,

    /// Number of log entries known to be committed (replicated on a
    /// majority). Entries `log[..commit_index]` are committed.
    pub(crate) commit_index: u64,

    /// Leader only: for each peer, the index of the next entry to send.
    pub(crate) next_index: HashMap<NodeId, u64>,

    /// Leader only: for each peer, the highest entry index known replicated.
    pub(crate) match_index: HashMap<NodeId, u64>,

    /// The leader this node currently recognizes (learned from
    /// AppendEntries). Used to forward proposals from non-leader nodes.
    pub leader_id: Option<NodeId>,
}

impl RaftState {
    /// Create a new node starting as Follower.
    ///
    /// `election_timeout` is the number of ticks of silence before this
    /// node starts an election. Callers should randomize this per node
    /// (e.g. `base + rng.gen_range(0..jitter)`) to avoid split votes.
    pub fn new(
        node_id: NodeId,
        peers: Vec<NodeId>,
        election_timeout: u64,
        heartbeat_interval: u64,
    ) -> Self {
        assert!(election_timeout > 0, "election_timeout must be > 0");
        assert!(heartbeat_interval > 0, "heartbeat_interval must be > 0");
        Self {
            node_id,
            peers,
            role: Role::Follower,
            current_term: Term::ZERO,
            voted_for: None,
            election_elapsed: 0,
            election_timeout,
            heartbeat_elapsed: 0,
            heartbeat_interval,
            votes_received: HashSet::new(),
            log: Vec::new(),
            commit_index: 0,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            leader_id: None,
        }
    }

    /// Like [`Self::new`], but the election timeout is randomized as
    /// `base + rng.gen_range(0..jitter)` ticks (ADR-0014 §5).
    ///
    /// Randomizing the timeout per node avoids repeated split votes when
    /// multiple Followers time out simultaneously.
    pub fn new_randomized(
        node_id: NodeId,
        peers: Vec<NodeId>,
        base_election_timeout: u64,
        jitter: u64,
        heartbeat_interval: u64,
        rng: &mut impl rand::Rng,
    ) -> Self {
        let election_timeout = base_election_timeout + rng.gen_range(0..jitter.max(1));
        Self::new(node_id, peers, election_timeout, heartbeat_interval)
    }

    /// Number of nodes required for a majority, including self.
    fn majority(&self) -> usize {
        (self.peers.len() + 1) / 2 + 1
    }

    /// Reset the election timer. Called when valid AppendEntries or
    /// RequestVote-granted communication is received from the current leader.
    pub fn reset_election_timer(&mut self) {
        self.election_elapsed = 0;
    }

    /// Advance the state machine by one logical Tick (ADR-0014 §7 Step 10).
    ///
    /// Returns the side effects the caller must perform this tick, if any.
    pub fn on_tick(&mut self) -> Vec<TickEffect> {
        let mut effects = Vec::new();

        match self.role {
            Role::Follower | Role::Candidate => {
                self.election_elapsed += 1;
                if self.election_elapsed >= self.election_timeout {
                    self.become_candidate();
                    effects.push(TickEffect::StartElection {
                        term: self.current_term,
                    });
                }
            }
            Role::Leader => {
                self.heartbeat_elapsed += 1;
                if self.heartbeat_elapsed >= self.heartbeat_interval {
                    self.heartbeat_elapsed = 0;
                    effects.push(TickEffect::SendHeartbeat {
                        term: self.current_term,
                    });
                }
            }
        }

        effects
    }

    /// Transition to Candidate: increment term, vote for self, reset election timer.
    ///
    /// In a single-node cluster the self-vote already constitutes a
    /// majority, so the node becomes Leader immediately.
    fn become_candidate(&mut self) {
        self.role = Role::Candidate;
        self.current_term = self.current_term.next();
        self.voted_for = Some(self.node_id);
        self.votes_received.clear();
        self.votes_received.insert(self.node_id);
        self.election_elapsed = 0;
        if self.votes_received.len() >= self.majority() {
            self.become_leader();
        }
    }

    /// Transition to Follower for `term`. Used when a higher term is observed
    /// from RequestVote or AppendEntries (rpc.rs).
    pub fn become_follower(&mut self, term: Term) {
        self.role = Role::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.votes_received.clear();
        self.election_elapsed = 0;
        self.leader_id = None;
    }

    /// Transition to Leader. Only valid from Candidate after winning a
    /// majority of votes (rpc.rs calls `record_vote` then this).
    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.heartbeat_elapsed = 0;
        self.votes_received.clear();
        self.leader_id = Some(self.node_id);
        // Standard Raft leader-volatile-state initialization.
        let last = self.log.len() as u64;
        self.next_index = self.peers.iter().map(|&p| (p, last)).collect();
        self.match_index = self.peers.iter().map(|&p| (p, 0)).collect();
    }

    /// Record a vote received from `voter` for the current term.
    /// Returns `true` if this vote caused a transition to Leader.
    ///
    /// No-op (returns `false`) if this node is not a Candidate or the vote
    /// is for a stale term.
    pub fn record_vote(&mut self, voter: NodeId, term: Term) -> bool {
        if self.role != Role::Candidate || term != self.current_term {
            return false;
        }
        self.votes_received.insert(voter);
        if self.votes_received.len() >= self.majority() {
            self.become_leader();
            return true;
        }
        false
    }

    // ── Log replication (ADR-0014 §3) ─────────────────────────────────────────

    /// Append a proposal to the Raft Log. Leader only.
    ///
    /// Returns `true` if the entry was appended; `false` if this node is not
    /// the Leader (the caller should forward the proposal to `leader_id`).
    pub fn propose(&mut self, payload: Vec<u8>) -> bool {
        if self.role != Role::Leader {
            return false;
        }
        self.log.push(LogEntry {
            term: self.current_term,
            payload,
        });
        // A lone-node "cluster" (no peers) commits immediately.
        self.recompute_commit_index();
        true
    }

    /// Number of committed log entries.
    pub fn commit_index(&self) -> u64 {
        self.commit_index
    }

    /// The committed entries in `[from..commit_index)`, for the caller to
    /// apply to its own state machine.
    pub fn committed_from(&self, from: u64) -> &[LogEntry] {
        &self.log[from as usize..self.commit_index as usize]
    }

    /// Leader only: the log suffix to send to `peer`, as
    /// `(prev_log_index, prev_log_term, entries)`.
    pub fn entries_for(&self, peer: NodeId) -> (u64, Term, Vec<LogEntry>) {
        let next = self
            .next_index
            .get(&peer)
            .copied()
            .unwrap_or(self.log.len() as u64);
        let prev_term = if next == 0 {
            Term::ZERO
        } else {
            self.log[next as usize - 1].term
        };
        (next, prev_term, self.log[next as usize..].to_vec())
    }

    /// Leader only: record a successful replication response from `peer`
    /// reporting its log length, then advance `commit_index` if a majority
    /// has replicated further. Returns `true` if `commit_index` advanced.
    pub fn record_replication(&mut self, peer: NodeId, peer_match: u64) -> bool {
        if self.role != Role::Leader {
            return false;
        }
        self.match_index.insert(peer, peer_match);
        self.next_index.insert(peer, peer_match);
        self.recompute_commit_index()
    }

    /// Leader only: a peer rejected AppendEntries due to log inconsistency.
    /// Back up `next_index` so the next heartbeat retries from earlier.
    pub fn record_rejection(&mut self, peer: NodeId, peer_log_len: u64) {
        if self.role != Role::Leader {
            return;
        }
        let next = self.next_index.entry(peer).or_insert(0);
        *next = (*next).saturating_sub(1).min(peer_log_len);
    }

    /// Advance `commit_index` to the highest index replicated on a majority,
    /// restricted to entries of the current term (Raft Figure 8 safety rule).
    fn recompute_commit_index(&mut self) -> bool {
        let mut matches: Vec<u64> = self.match_index.values().copied().collect();
        matches.push(self.log.len() as u64); // self
        matches.sort_unstable_by(|a, b| b.cmp(a));
        let majority_match = matches[self.majority() - 1];

        if majority_match > self.commit_index
            && self.log[majority_match as usize - 1].term == self.current_term
        {
            self.commit_index = majority_match;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u8) -> NodeId {
        NodeId(id)
    }

    fn three_node_cluster() -> RaftState {
        RaftState::new(node(0), vec![node(1), node(2)], 5, 1)
    }

    #[test]
    fn new_node_starts_as_follower_in_term_zero() {
        let state = three_node_cluster();
        assert_eq!(state.role, Role::Follower);
        assert_eq!(state.current_term, Term::ZERO);
        assert_eq!(state.voted_for, None);
    }

    #[test]
    fn follower_becomes_candidate_after_election_timeout_ticks_elapse() {
        let mut state = three_node_cluster();
        for _ in 0..4 {
            let effects = state.on_tick();
            assert!(effects.is_empty());
            assert_eq!(state.role, Role::Follower);
        }
        let effects = state.on_tick();
        assert_eq!(state.role, Role::Candidate);
        assert_eq!(state.current_term, Term(1));
        assert_eq!(effects, vec![TickEffect::StartElection { term: Term(1) }]);
    }

    #[test]
    fn candidate_votes_for_itself_on_becoming_candidate() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        assert_eq!(state.voted_for, Some(state.node_id));
    }

    #[test]
    fn reset_election_timer_prevents_timeout() {
        let mut state = three_node_cluster();
        for _ in 0..4 {
            state.on_tick();
        }
        state.reset_election_timer();
        let effects = state.on_tick();
        assert!(effects.is_empty());
        assert_eq!(state.role, Role::Follower);
    }

    #[test]
    fn candidate_becomes_leader_after_receiving_majority_of_votes() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        assert_eq!(state.role, Role::Candidate);
        let term = state.current_term;

        // Self-vote already counted; one more vote reaches majority of 2/3.
        let became_leader = state.record_vote(node(1), term);
        assert!(became_leader);
        assert_eq!(state.role, Role::Leader);
    }

    #[test]
    fn vote_for_stale_term_is_ignored() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        let stale_term = Term(0);
        let became_leader = state.record_vote(node(1), stale_term);
        assert!(!became_leader);
        assert_eq!(state.role, Role::Candidate);
    }

    #[test]
    fn leader_sends_heartbeat_every_heartbeat_interval_ticks() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        state.record_vote(node(1), state.current_term);
        assert_eq!(state.role, Role::Leader);

        let effects = state.on_tick();
        assert_eq!(effects, vec![TickEffect::SendHeartbeat { term: Term(1) }]);
    }

    #[test]
    fn become_follower_resets_vote_and_election_timer() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        assert_eq!(state.role, Role::Candidate);

        state.become_follower(Term(5));
        assert_eq!(state.role, Role::Follower);
        assert_eq!(state.current_term, Term(5));
        assert_eq!(state.voted_for, None);

        // Election timer was reset; needs full timeout again.
        for _ in 0..4 {
            let effects = state.on_tick();
            assert!(effects.is_empty());
        }
        let effects = state.on_tick();
        assert_eq!(effects, vec![TickEffect::StartElection { term: Term(6) }]);
    }

    #[test]
    fn randomized_election_timeout_falls_within_base_plus_jitter_range() {
        let mut rng = rand::thread_rng();
        for _ in 0..50 {
            let mut state =
                RaftState::new_randomized(node(0), vec![node(1), node(2)], 10, 5, 1, &mut rng);
            // election_timeout is private; observe it indirectly via on_tick.
            let mut ticks = 0;
            loop {
                let effects = state.on_tick();
                ticks += 1;
                if !effects.is_empty() {
                    break;
                }
            }
            assert!((10..15).contains(&ticks), "ticks={ticks} out of [10,15)");
        }
    }

    #[test]
    fn term_next_increments_by_one() {
        assert_eq!(Term::ZERO.next(), Term(1));
        assert_eq!(Term(5).next(), Term(6));
    }

    // ── Log replication (ADR-0014 §3) ─────────────────────────────────────────

    fn make_leader(state: &mut RaftState) {
        for _ in 0..5 {
            state.on_tick();
        }
        let term = state.current_term;
        state.record_vote(node(1), term);
        assert_eq!(state.role, Role::Leader);
    }

    #[test]
    fn propose_is_rejected_when_node_is_not_leader() {
        let mut state = three_node_cluster();
        assert!(!state.propose(b"p".to_vec()));
        assert_eq!(state.commit_index(), 0);
    }

    #[test]
    fn leader_appends_proposal_to_its_log_without_committing_until_majority() {
        let mut state = three_node_cluster();
        make_leader(&mut state);
        assert!(state.propose(b"p1".to_vec()));
        // 3-node cluster: self alone is not a majority.
        assert_eq!(state.commit_index(), 0);
    }

    #[test]
    fn commit_index_advances_once_a_majority_has_replicated() {
        let mut state = three_node_cluster();
        make_leader(&mut state);
        state.propose(b"p1".to_vec());

        let advanced = state.record_replication(node(1), 1);
        assert!(advanced, "self + one peer = majority of 3");
        assert_eq!(state.commit_index(), 1);
        assert_eq!(state.committed_from(0).len(), 1);
        assert_eq!(state.committed_from(0)[0].payload, b"p1");
    }

    #[test]
    fn lone_node_with_no_peers_commits_its_own_proposal_immediately() {
        let mut state = RaftState::new(node(0), vec![], 5, 1);
        for _ in 0..5 {
            state.on_tick();
        }
        // Single-node cluster: the self-vote is a majority, so the node
        // becomes Leader at election timeout and commits without peers.
        assert_eq!(state.role, Role::Leader);
        assert!(state.propose(b"p".to_vec()));
        assert_eq!(state.commit_index(), 1);
    }

    #[test]
    fn entries_for_returns_the_suffix_a_peer_is_missing() {
        let mut state = three_node_cluster();
        make_leader(&mut state);
        state.propose(b"p1".to_vec());
        state.propose(b"p2".to_vec());

        let (prev, prev_term, entries) = state.entries_for(node(1));
        assert_eq!(prev, 0, "fresh peer starts from the beginning");
        assert_eq!(prev_term, Term::ZERO);
        assert_eq!(entries.len(), 2);

        state.record_replication(node(1), 2);
        let (prev, _, entries) = state.entries_for(node(1));
        assert_eq!(prev, 2);
        assert!(entries.is_empty(), "up-to-date peer gets a pure heartbeat");
    }

    #[test]
    fn record_rejection_backs_off_next_index_for_the_peer() {
        let mut state = three_node_cluster();
        make_leader(&mut state);
        state.propose(b"p1".to_vec());
        state.propose(b"p2".to_vec());
        state.record_replication(node(1), 2);

        state.record_rejection(node(1), 0);
        let (prev, _, entries) = state.entries_for(node(1));
        assert_eq!(prev, 0, "next_index backed off to the peer's log length");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn commit_is_restricted_to_entries_of_the_current_term() {
        let mut state = three_node_cluster();
        make_leader(&mut state);
        state.propose(b"old".to_vec());

        // A new term begins (e.g. re-election) before the entry replicates.
        let new_term = state.current_term.next().next();
        state.become_follower(new_term);
        for _ in 0..5 {
            state.on_tick();
        }
        state.record_vote(node(1), state.current_term);
        assert_eq!(state.role, Role::Leader);

        // The old-term entry alone must not commit even with majority match
        // (Raft Figure 8 safety rule)...
        assert!(!state.record_replication(node(1), 1));
        assert_eq!(state.commit_index(), 0);

        // ...but committing a current-term entry commits everything before it.
        state.propose(b"new".to_vec());
        assert!(state.record_replication(node(1), 2));
        assert_eq!(state.commit_index(), 2);
    }
}
