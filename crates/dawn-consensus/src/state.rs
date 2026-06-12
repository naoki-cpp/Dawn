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
use std::collections::HashSet;

/// Raft term number. Monotonically increasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize)]
pub struct Term(pub u64);

impl Term {
    pub const ZERO: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
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
}

impl RaftState {
    /// Create a new node starting as Follower.
    ///
    /// `election_timeout` is the number of ticks of silence before this
    /// node starts an election. Callers should randomize this per node
    /// (e.g. `base + rng.gen_range(0..jitter)`) to avoid split votes.
    pub fn new(node_id: NodeId, peers: Vec<NodeId>, election_timeout: u64, heartbeat_interval: u64) -> Self {
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
                    effects.push(TickEffect::StartElection { term: self.current_term });
                }
            }
            Role::Leader => {
                self.heartbeat_elapsed += 1;
                if self.heartbeat_elapsed >= self.heartbeat_interval {
                    self.heartbeat_elapsed = 0;
                    effects.push(TickEffect::SendHeartbeat { term: self.current_term });
                }
            }
        }

        effects
    }

    /// Transition to Candidate: increment term, vote for self, reset election timer.
    fn become_candidate(&mut self) {
        self.role = Role::Candidate;
        self.current_term = self.current_term.next();
        self.voted_for = Some(self.node_id);
        self.votes_received.clear();
        self.votes_received.insert(self.node_id);
        self.election_elapsed = 0;
    }

    /// Transition to Follower for `term`. Used when a higher term is observed
    /// from RequestVote or AppendEntries (rpc.rs).
    pub fn become_follower(&mut self, term: Term) {
        self.role = Role::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.votes_received.clear();
        self.election_elapsed = 0;
    }

    /// Transition to Leader. Only valid from Candidate after winning a
    /// majority of votes (rpc.rs calls `record_vote` then this).
    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.heartbeat_elapsed = 0;
        self.votes_received.clear();
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
            let mut state = RaftState::new_randomized(node(0), vec![node(1), node(2)], 10, 5, 1, &mut rng);
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
}
