//! Sector Transit Raft adapter (ADR-0014).
//!
//! The durable recovery and idempotency policy lives in the internal pipeline
//! module. This module owns only the wire payload and the translation between
//! committed Raft entries and pipeline proposals.

pub(crate) mod pipeline;

use crate::node::SimulationNode;
use dawn_consensus::RaftActorHandle;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, Position, SectorId, ShipId, Tick,
    TransitHandoffState,
};
use dawn_event_store::store::EventStore;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitOp {
    Request {
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    },
    Commit {
        handoff: Box<TransitHandoffState>,
        from: SectorId,
        to: SectorId,
        entry_pos: Position,
        entry_pos_abs: AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    },
    Ack {
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: Tick,
    },
}

impl TransitOp {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("TransitOp serialization cannot fail")
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        postcard::from_bytes(payload).ok()
    }
}

fn propose_commit(raft: &RaftActorHandle, proposal: pipeline::CommitProposal) {
    raft.propose(
        TransitOp::Commit {
            handoff: Box::new(proposal.handoff),
            from: proposal.from,
            to: proposal.to,
            entry_pos: proposal.entry_pos,
            entry_pos_abs: proposal.entry_pos_abs,
            gate_id: proposal.gate_id,
            request_tick: proposal.request_tick,
        }
        .encode(),
    );
}

fn propose_ack(raft: &RaftActorHandle, proposal: pipeline::AckProposal) {
    raft.propose(
        TransitOp::Ack {
            ship_id: proposal.ship_id,
            from: proposal.from,
            to: proposal.to,
            request_tick: proposal.request_tick,
        }
        .encode(),
    );
}

pub fn apply_committed_raft_entries<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Ok(payload) = committed_rx.try_recv() {
        let Some(op) = TransitOp::decode(&payload) else {
            continue;
        };
        match op {
            TransitOp::Request {
                ship_id,
                to,
                gate_id,
            } => {
                if let Some(proposal) = pipeline::apply_request(node, ship_id, to, gate_id) {
                    propose_commit(raft, proposal);
                }
            }
            TransitOp::Commit {
                handoff,
                from,
                to,
                entry_pos,
                entry_pos_abs,
                gate_id,
                request_tick,
            } => {
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &handoff,
                    from,
                    to,
                    entry_pos,
                    entry_pos_abs,
                    gate_id,
                    request_tick,
                ) {
                    propose_ack(raft, proposal);
                }
            }
            TransitOp::Ack {
                ship_id,
                from,
                to,
                request_tick,
            } => {
                pipeline::apply_ack(node, ship_id, from, to, request_tick);
            }
        }
    }

    for proposal in pipeline::due_retries(node) {
        propose_commit(raft, proposal);
    }
}

#[derive(Debug)]
pub struct RuntimeTickOutput {
    pub tick_result: crate::node::TickResult,
    pub events: Vec<DomainEvent>,
    /// Auto-jump triggers that passed final validation and were proposed to Raft.
    /// Rejected one-shot triggers are drained but deliberately omitted.
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub completed_warps: Vec<ShipId>,
}

/// Execute the authoritative server frame pipeline.
///
/// Ordering is deliberately centralized here for every runtime adapter:
/// committed Raft entries -> simulation Tick -> Event collection ->
/// replication hook -> Raft clock advancement -> auto-jump proposal ->
/// transient warp-output drain.
pub fn run_runtime_tick<S, F>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_collected: F,
) -> RuntimeTickOutput
where
    S: EventStore,
    F: FnOnce(&SimulationNode<S>, &crate::node::TickResult, &[DomainEvent]),
{
    let events_before = node.total_event_count() as u64;
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    let events: Vec<_> = node
        .event_store()
        .iter_from(events_before)
        .map(|record| record.event.clone())
        .collect();

    // Replication must observe the newly appended Event tail before the
    // consensus clock advances. The immutable node reference keeps this hook
    // publication-only so the collected output cannot diverge from the log.
    after_events_collected(node, &result, &events);
    raft.tick();

    // Auto-jump is a simulation transient, not adapter-owned Tick ordering.
    // Drain and propose it here so actor, clustered serve, and production Node
    // paths all complete the same-frame handoff. Only successful proposals are
    // surfaced to adapters; rejected one-shot attempts retain their historical
    // silent-drop behavior.
    let pending_auto_jumps = node
        .drain_pending_auto_jumps()
        .into_iter()
        .filter_map(|(ship_id, gate_id)| {
            propose_auto_jump(node, raft, ship_id, gate_id).map(|_| (ship_id, gate_id))
        })
        .collect();
    let completed_warps = node.drain_completed_warps();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}

pub fn propose_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> crate::node::JumpOutcome {
    let outcome = node.apply_jump_with_fallback(ship_id, gate_id);
    if let crate::node::JumpOutcome::NeedsTransitProposal { to } = outcome {
        propose_transit_request(raft, ship_id, to, gate_id);
    }
    outcome
}

pub fn propose_auto_jump<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    ship_id: ShipId,
    gate_id: JumpGateId,
) -> Option<SectorId> {
    let to = node.resolve_auto_jump(ship_id, gate_id)?;
    propose_transit_request(raft, ship_id, to, gate_id);
    Some(to)
}

fn propose_transit_request(
    raft: &RaftActorHandle,
    ship_id: ShipId,
    to: SectorId,
    gate_id: JumpGateId,
) {
    raft.propose(
        TransitOp::Request {
            ship_id,
            to,
            gate_id: Some(gate_id),
        }
        .encode(),
    );
}

#[cfg(test)]
mod runtime_tick_review_tests;
#[cfg(test)]
mod tests;
