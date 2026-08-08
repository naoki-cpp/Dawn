//! Sector runtime adapters (ADR-0014 / ADR-0049).
//!
//! The authoritative handoff state machine lives in the internal `handoff`
//! module. The pipeline consumes the node's observed transit journal and
//! translates explicit effects to Raft proposals. The low-level ECS lifecycle
//! operations remain crate-private behind that policy. This module also owns
//! the runtime-side durable Stop/Tick transition functions so the
//! storage-independent `SimulationNode` only prepares and applies state.

pub(crate) mod handoff;
pub(crate) mod pipeline;

use crate::node::SimulationNode;
use dawn_consensus::RaftActorHandle;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, SectorId, ShipId, Tick, TransitHandoffState,
};
use dawn_event_store::{AppendReceipt, DurabilityMode, DurableJournal, JournalError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

/// Failure returned by the Stop prepare -> durable -> live-apply boundary.
#[derive(Debug, Error)]
pub enum StopTransitionError {
    #[error(transparent)]
    Preparation(#[from] crate::transition::TransitionError),
    #[error("durable transition append failed: {0}")]
    Durable(#[from] JournalError),
    #[error("prepared Stop cannot be applied to the current state: {0}")]
    Validation(#[from] crate::transition::TransitionApplyError),
}

/// Failure returned by the Tick prepare -> durable -> live-apply boundary.
#[derive(Debug, Error)]
pub enum TickTransitionError {
    #[error(transparent)]
    Preparation(#[from] crate::node::TickPreparationError),
    #[error("durable transition append failed: {0}")]
    Durable(#[from] JournalError),
    #[error("prepared Tick cannot be applied to the current state: {0}")]
    Validation(#[from] crate::transition::TransitionApplyError),
}

/// Commit a prepared Stop transition through the runtime-owned journal.
pub fn commit_stop_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    ship_id: ShipId,
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, StopTransitionError> {
    let prepared = node.prepare_stop_transition(ship_id, transition_id, owner_epoch)?;
    let delta = match &prepared.recovery_delta {
        crate::transition::SectorRecoveryDelta::Stop(delta) => *delta,
        crate::transition::SectorRecoveryDelta::Tick(_) => {
            unreachable!("prepare_stop_transition always produces a Stop delta")
        }
    };
    // Resolve the live entity before the durable append. Once the journal
    // accepts the transition, applying this already-validated entity is
    // infallible; a post-commit lookup failure must not masquerade as a
    // normal command rejection.
    let entity = node.stop_entity(delta.ship_id)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_stop_delta(entity, delta);
    Ok(receipt)
}

/// Commit the complete bounded ECS Tick write set through the runtime journal.
pub fn commit_tick_state_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    lock_commands: &[dawn_core::LockOnCommand],
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, TickTransitionError> {
    let (prepared, _) =
        node.prepare_tick_state_transition_with_result(lock_commands, transition_id, owner_epoch)?;
    let crate::transition::SectorRecoveryDelta::Tick(ref delta) = prepared.recovery_delta else {
        unreachable!("prepare_tick_state_transition always produces a Tick delta")
    };
    node.validate_tick_transition(delta, prepared.context)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_validated_full_tick(delta.clone())?;
    node.observe_committed_events(&prepared.public_events);
    Ok(receipt)
}

/// Commit the logical Tick counter through the runtime journal.
pub fn commit_tick_transition<J: DurableJournal>(
    node: &mut SimulationNode,
    journal: &mut J,
    transition_id: crate::transition::SectorTransitionId,
    owner_epoch: u64,
    durability: DurabilityMode,
) -> Result<AppendReceipt, TickTransitionError> {
    let prepared = node
        .prepare_tick_transition(transition_id, owner_epoch)
        .map_err(crate::node::TickPreparationError::from)?;
    let crate::transition::SectorRecoveryDelta::Tick(ref delta) = prepared.recovery_delta else {
        unreachable!("prepare_tick_transition always produces a Tick delta")
    };
    node.validate_tick_transition(delta, prepared.context)?;
    let receipt =
        crate::transition_journal::append_prepared_transition(journal, &prepared, durability)?;
    node.apply_validated_logical_tick(delta.clone());
    Ok(receipt)
}

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
        entry_pos: AbsolutePosition,
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

pub fn apply_committed_raft_entries(
    node: &mut SimulationNode,
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
                gate_id,
                request_tick,
            } => {
                if let Some(proposal) = pipeline::apply_commit(
                    node,
                    &handoff,
                    from,
                    to,
                    entry_pos,
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

#[derive(Debug, Clone, Copy)]
pub struct DurableRuntimeTickContext {
    pub transition_id: crate::transition::SectorTransitionId,
    pub owner_epoch: u64,
    pub durability: DurabilityMode,
}

/// Execute the authoritative server frame pipeline.
///
/// Ordering is deliberately centralized here for every runtime adapter:
/// committed Raft entries -> simulation Tick -> Event collection ->
/// replication hook -> Raft clock advancement -> auto-jump proposal ->
/// transient warp-output drain.
pub fn run_runtime_tick<F>(
    node: &mut SimulationNode,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_collected: F,
) -> RuntimeTickOutput
where
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    // Consume the engine's explicit transition output instead of deriving the
    // frame from a mutable public-event cursor. The legacy log remains a
    // mirror during migration, but runtime publication is driven by output.
    let mut events = node.drain_pending_events();
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    events.extend(node.drain_pending_events());

    // Replication must observe the newly produced transition output before the
    // consensus clock advances. The immutable node reference keeps this hook
    // publication-only so the collected output cannot diverge from the node.
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

/// Execute one frame with the ADR-0049 durable Tick boundary.
///
/// Unlike [`run_runtime_tick`], this path does not let the ECS Tick mutate the
/// live state before persistence. It prepares the bounded recovery write set,
/// appends the recovery/public batch to the supplied journal, applies the same
/// delta, and only then publishes the public events and transient effects.
/// The legacy frame remains available while callers migrate their journal
/// wiring to this explicit runtime-owned path.
pub fn run_durable_runtime_tick<J, F>(
    node: &mut SimulationNode,
    journal: &mut J,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    context: DurableRuntimeTickContext,
    after_events_collected: F,
) -> Result<RuntimeTickOutput, TickTransitionError>
where
    J: DurableJournal,
    F: FnOnce(&SimulationNode, &crate::node::TickResult, &[DomainEvent]),
{
    apply_committed_raft_entries(node, raft, committed_rx);
    // Command-side and committed-transit public facts belong to this same
    // runtime output boundary. Keep them outside the prepared Tick until the
    // journal append has succeeded so a failed append can restore the buffer.
    let prior_events = node.drain_pending_events();
    let (prepared, result) = match node.prepare_tick_state_transition_with_result(
        lock_commands,
        context.transition_id,
        context.owner_epoch,
    ) {
        Ok(result) => result,
        Err(error) => {
            node.restore_pending_events(prior_events);
            return Err(TickTransitionError::Preparation(error));
        }
    };
    let delta = match &prepared.recovery_delta {
        crate::transition::SectorRecoveryDelta::Tick(delta) => delta.clone(),
        crate::transition::SectorRecoveryDelta::Stop(_) => {
            unreachable!("full runtime Tick preparation produces a Tick delta")
        }
    };
    if let Err(error) = crate::transition_journal::append_prepared_transition(
        journal,
        &prepared,
        context.durability,
    ) {
        node.restore_pending_events(prior_events);
        return Err(TickTransitionError::Durable(error));
    }
    node.apply_tick_transition(delta, prepared.context)
        .map_err(TickTransitionError::Validation)?;
    node.observe_committed_events(&prepared.public_events);
    let mut events = prior_events;
    events.extend(node.drain_pending_events());
    events.extend(prepared.public_events);

    after_events_collected(node, &result, &events);
    raft.tick();

    let pending_auto_jumps = node
        .drain_pending_auto_jumps()
        .into_iter()
        .filter_map(|(ship_id, gate_id)| {
            propose_auto_jump(node, raft, ship_id, gate_id).map(|_| (ship_id, gate_id))
        })
        .collect();
    let completed_warps = node.drain_completed_warps();

    Ok(RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    })
}

pub fn propose_jump(
    node: &mut SimulationNode,
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

pub fn propose_auto_jump(
    node: &mut SimulationNode,
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
mod handoff_tests;
#[cfg(test)]
mod runtime_tick_review_tests;
#[cfg(test)]
mod tests;
