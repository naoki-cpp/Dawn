//! Sector Transit EventStore and consensus adapter (ADR-0014).
//!
//! This module reconstructs durable Transit facts and translates the deep
//! handoff module's explicit effects into Raft proposal payloads. It does not
//! decide state transitions, idempotency, retry policy, or cleanup behavior.

use super::handoff;
use crate::node::SimulationNode;
use dawn_core::{
    AbsolutePosition, DomainEvent, JumpGateId, SectorId, ShipId, Tick, TransitHandoffState,
};
use dawn_event_store::store::EventStore;

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

pub(crate) use handoff::ReplayDirective;

pub(crate) fn replay_directive(event: &DomainEvent) -> Option<ReplayDirective<'_>> {
    handoff::replay_directive(event)
}

fn journal<S: EventStore>(node: &SimulationNode<S>) -> handoff::TransitJournal {
    let mut journal = handoff::TransitJournal::new(node.sector_id());
    for record in node.event_store().iter_from(0) {
        journal.observe(&record.event);
    }
    journal
}

/// Whether checkpoint compaction must be deferred to preserve the durable
/// outgoing request used for restart retry.
pub(crate) fn has_pending_outgoing_transit<S: EventStore>(node: &SimulationNode<S>) -> bool {
    handoff::has_pending_outgoing_transit(&journal(node))
}

pub(crate) fn apply_request<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    to: SectorId,
    gate_id: Option<JumpGateId>,
) -> Option<CommitProposal> {
    handoff::apply_request(node, ship_id, to, gate_id).map(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_commit<S: EventStore>(
    node: &mut SimulationNode<S>,
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

pub(crate) fn apply_ack<S: EventStore>(
    node: &mut SimulationNode<S>,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
    request_tick: Tick,
) -> bool {
    let journal = journal(node);
    handoff::apply_ack(node, &journal, ship_id, from, to, request_tick)
}

pub(crate) fn due_retries<S: EventStore>(node: &mut SimulationNode<S>) -> Vec<CommitProposal> {
    let journal = journal(node);
    handoff::due_retries(node, &journal)
        .into_iter()
        .map(Into::into)
        .collect()
}
