//! RequestVote / AppendEntries message types and their effect on
//! [`RaftState`].
//!
//! Message handling here only concerns the *election* aspects of Raft
//! (term/role transitions, vote granting, leader recognition). Log entry
//! replication payloads are added when `log.rs` is introduced.

use crate::state::{RaftState, Role, Term};
use dawn_core::NodeId;
use serde::{Deserialize, Serialize};

/// A vote request sent by a Candidate to a peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestVote {
    pub term        : Term,
    pub candidate_id: NodeId,
}

/// A peer's response to [`RequestVote`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestVoteResponse {
    pub term        : Term,
    pub vote_granted: bool,
    pub voter       : NodeId,
}

/// A heartbeat / log-replication message sent by the Leader.
///
/// `entries` is left empty for pure heartbeats. Log entry payloads are
/// added when `log.rs` is introduced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppendEntries {
    pub term     : Term,
    pub leader_id: NodeId,
}

/// A follower's response to [`AppendEntries`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    pub term     : Term,
    pub success  : bool,
    pub responder: NodeId,
}

/// Messages exchanged between `RaftActor`s via `RaftTransport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RaftMessage {
    RequestVote(RequestVote),
    RequestVoteResponse(RequestVoteResponse),
    AppendEntries(AppendEntries),
    AppendEntriesResponse(AppendEntriesResponse),
}

impl RaftState {
    /// Handle an incoming [`RequestVote`] RPC.
    ///
    /// Grants the vote iff:
    /// - `req.term >= self.current_term` (stepping down to Follower if `>`)
    /// - this node has not already voted in `req.term`, or already voted
    ///   for `req.candidate_id`
    ///
    /// Granting a vote resets the election timer (the candidate is alive).
    pub fn handle_request_vote(&mut self, req: &RequestVote) -> RequestVoteResponse {
        if req.term > self.current_term {
            self.become_follower(req.term);
        }

        let already_voted_for_other =
            matches!(self.voted_for, Some(v) if v != req.candidate_id);

        let vote_granted = req.term == self.current_term && !already_voted_for_other;

        if vote_granted {
            self.voted_for = Some(req.candidate_id);
            self.reset_election_timer();
        }

        RequestVoteResponse {
            term: self.current_term,
            vote_granted,
            voter: self.node_id,
        }
    }

    /// Handle an incoming [`AppendEntries`] RPC (heartbeat or log replication).
    ///
    /// Rejects if `req.term < self.current_term`. Otherwise recognizes
    /// `req.leader_id` as the current leader: steps down to Follower (if
    /// Candidate/Leader or a higher term is observed) and resets the
    /// election timer.
    pub fn handle_append_entries(&mut self, req: &AppendEntries) -> AppendEntriesResponse {
        if req.term < self.current_term {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                responder: self.node_id,
            };
        }

        if req.term > self.current_term || self.role != Role::Follower {
            self.become_follower(req.term);
        }
        self.reset_election_timer();

        AppendEntriesResponse {
            term: self.current_term,
            success: true,
            responder: self.node_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TickEffect;

    fn node(id: u8) -> NodeId {
        NodeId(id)
    }

    fn three_node_cluster() -> RaftState {
        RaftState::new(node(0), vec![node(1), node(2)], 5, 1)
    }

    #[test]
    fn follower_grants_vote_for_higher_term_candidate() {
        let mut state = three_node_cluster();
        let resp = state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(1) });
        assert!(resp.vote_granted);
        assert_eq!(resp.term, Term(1));
        assert_eq!(state.current_term, Term(1));
        assert_eq!(state.voted_for, Some(node(1)));
    }

    #[test]
    fn follower_rejects_second_vote_request_in_same_term() {
        let mut state = three_node_cluster();
        state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(1) });

        let resp = state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(2) });
        assert!(!resp.vote_granted);
        assert_eq!(state.voted_for, Some(node(1)));
    }

    #[test]
    fn node_re_grants_vote_to_same_candidate_in_same_term() {
        let mut state = three_node_cluster();
        state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(1) });
        let resp = state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(1) });
        assert!(resp.vote_granted);
    }

    #[test]
    fn vote_request_for_stale_term_is_rejected() {
        let mut state = three_node_cluster();
        // Bring the node to term 5.
        state.become_follower(Term(5));

        let resp = state.handle_request_vote(&RequestVote { term: Term(3), candidate_id: node(1) });
        assert!(!resp.vote_granted);
        assert_eq!(resp.term, Term(5));
        assert_eq!(state.current_term, Term(5));
    }

    #[test]
    fn granting_a_vote_resets_the_election_timer() {
        let mut state = three_node_cluster();
        for _ in 0..4 {
            state.on_tick();
        }
        state.handle_request_vote(&RequestVote { term: Term(1), candidate_id: node(1) });

        // Timer was reset; 4 more ticks should not trigger an election.
        for _ in 0..4 {
            let effects = state.on_tick();
            assert!(effects.is_empty());
        }
    }

    #[test]
    fn follower_accepts_heartbeat_from_leader_with_current_term() {
        let mut state = three_node_cluster();
        state.become_follower(Term(1));

        let resp = state.handle_append_entries(&AppendEntries { term: Term(1), leader_id: node(1) });
        assert!(resp.success);
        assert_eq!(resp.term, Term(1));
    }

    #[test]
    fn append_entries_with_stale_term_is_rejected() {
        let mut state = three_node_cluster();
        state.become_follower(Term(5));

        let resp = state.handle_append_entries(&AppendEntries { term: Term(3), leader_id: node(1) });
        assert!(!resp.success);
        assert_eq!(resp.term, Term(5));
        assert_eq!(state.current_term, Term(5));
    }

    #[test]
    fn candidate_steps_down_when_append_entries_received_for_current_term() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        assert_eq!(state.role, Role::Candidate);
        let term = state.current_term;

        let resp = state.handle_append_entries(&AppendEntries { term, leader_id: node(1) });
        assert!(resp.success);
        assert_eq!(state.role, Role::Follower);
    }

    #[test]
    fn append_entries_with_higher_term_causes_leader_to_step_down() {
        let mut state = three_node_cluster();
        for _ in 0..5 {
            state.on_tick();
        }
        let term = state.current_term;
        state.record_vote(node(1), term);
        assert_eq!(state.role, Role::Leader);

        let resp = state.handle_append_entries(&AppendEntries { term: term.next(), leader_id: node(1) });
        assert!(resp.success);
        assert_eq!(state.role, Role::Follower);
        assert_eq!(state.current_term, term.next());
    }

    #[test]
    fn heartbeat_resets_election_timer_preventing_new_election() {
        let mut state = three_node_cluster();
        state.become_follower(Term(1));
        for _ in 0..4 {
            assert!(state.on_tick().is_empty());
        }
        state.handle_append_entries(&AppendEntries { term: Term(1), leader_id: node(1) });

        for _ in 0..4 {
            let effects = state.on_tick();
            assert!(effects.is_empty());
        }
        assert_eq!(state.role, Role::Follower);
    }

    #[test]
    fn full_election_cycle_via_rpc_messages() {
        // Node 0 times out and becomes Candidate.
        let mut n0 = RaftState::new(node(0), vec![node(1), node(2)], 5, 1);
        for _ in 0..5 {
            n0.on_tick();
        }
        assert_eq!(n0.role, Role::Candidate);
        let term = n0.current_term;

        // Peers grant their votes.
        let mut n1 = RaftState::new(node(1), vec![node(0), node(2)], 10, 1);
        let mut n2 = RaftState::new(node(2), vec![node(0), node(1)], 10, 1);

        let resp1 = n1.handle_request_vote(&RequestVote { term, candidate_id: node(0) });
        let resp2 = n2.handle_request_vote(&RequestVote { term, candidate_id: node(0) });
        assert!(resp1.vote_granted);
        assert!(resp2.vote_granted);

        let became_leader = n0.record_vote(resp1.voter, resp1.term);
        assert!(became_leader);
        assert_eq!(n0.role, Role::Leader);

        // Leader sends heartbeat; followers accept it.
        let effects = n0.on_tick();
        assert_eq!(effects, vec![TickEffect::SendHeartbeat { term }]);

        let hb = AppendEntries { term, leader_id: node(0) };
        assert!(n1.handle_append_entries(&hb).success);
        assert!(n2.handle_append_entries(&hb).success);
    }
}
