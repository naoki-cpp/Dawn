//! Sector Transit output and consensus adapter (ADR-0014).
//!
//! This module reads the node-local transit journal and translates the deep
//! handoff module's explicit effects into Raft proposal payloads. It does not
//! decide state transitions, idempotency, retry policy, or cleanup behavior.

use super::handoff;
use crate::node::SimulationNode;
#[cfg(test)]
use dawn_core::DomainEvent;
use dawn_core::{AbsolutePosition, JumpGateId, SectorId, ShipId, Tick, TransitHandoffState};

#[derive(Debug)]
pub(crate) struct CommitProposal {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub entry_pos: AbsolutePosition,
    pub gate_id: Option<JumpGateId>,
    pub request_tick: Tick,
}

impl From<handoff::CommitEffect> for CommitProposal {
    fn from(effect: handoff::CommitEffect) -> Self {
        Self {
            handoff: effect.handoff,
            from: effect.from,
            to: effect.to,
            entry_pos: effect.entry_pos,
            gate_id: effect.gate_id,
            request_tick: effect.request_tick,
        }
    }
}

#[derive(Debug)]
pub(crate) struct AckProposal {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

impl From<handoff::AckEffect> for AckProposal {
    fn from(effect: handoff::AckEffect) -> Self {
        Self {
            ship_id: effect.ship_id,
            from: effect.from,
            to: effect.to,
            request_tick: effect.request_tick,
        }
    }
}

#[cfg(test)]
pub(crate) use handoff::ReplayDirective;

#[cfg(test)]
pub(crate) fn replay_directive(event: &DomainEvent) -> Option<ReplayDirective<'_>> {
    handoff::replay_directive(event)
}

fn journal(node: &SimulationNode) -> handoff::TransitJournal {
    node.transit_journal().clone()
}

/// Whether checkpoint compaction must be deferred to preserve the durable
/// outgoing request used for restart retry.
pub(crate) fn has_pending_outgoing_transit(node: &SimulationNode) -> bool {
    handoff::has_pending_outgoing_transit(&journal(node))
}

pub(crate) fn apply_request(
    node: &mut SimulationNode,
    ship_id: ShipId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> Option<CommitProposal> {
    handoff::apply_request(node, ship_id, to, gate_id).map(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_commit(
    node: &mut SimulationNode,
    handoff_state: &TransitHandoffState,
    from: SectorId,
    to: SectorId,
    entry_pos: AbsolutePosition,
    gate_id: Option<JumpGateId>,
    request_tick: Tick,
) -> Option<AckProposal> {
    let journal = journal(node);
    handoff::apply_commit(
        node,
        &journal,
        handoff_state,
        from,
        to,
        entry_pos,
        gate_id,
        request_tick,
    )
    .map(Into::into)
}

pub(crate) fn apply_ack(
    node: &mut SimulationNode,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    let journal = journal(node);
    handoff::apply_ack(node, &journal, ship_id, from, to, request_tick)
}

pub(crate) fn due_retries(node: &mut SimulationNode) -> Vec<CommitProposal> {
    let journal = journal(node);
    handoff::due_retries(node, &journal)
        .into_iter()
        .map(Into::into)
        .collect()
}
