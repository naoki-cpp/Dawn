//! Live Transit lifecycle mutation for `SimulationNode`.

use crate::persistence::{
    IncomingTransitReceipt, OutgoingTransitAttempt, TransitAttemptState, TransitSagaDiagnostics,
    TransitSagaSnapshot,
};
use dawn_core::{
    commands::TransitCommand,
    events::{JumpGateUsed, SectorTransitCompleted, SectorTransitRequested, StarSystemChanged},
    fitting::FittingSnapshot,
    DawnError, DomainEvent, JumpGateId, SectorId, ShipId, ShipTypeId, Tick, TransitAttemptId,
    TransitHandoffState,
};
use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp, InventoryComp, VelocityComp};
use dawn_ecs::TransitState;

use super::super::SimulationNode;

/// Everything the Transit handoff boundary needs to propose a
/// `TransitOp::Commit`, produced by [`SimulationNode::prepare_transit_commit`].
#[derive(Debug)]
pub(crate) struct TransitCommitData {
    pub(crate) attempt_id: TransitAttemptId,
    pub(crate) handoff: Box<TransitHandoffState>,
    pub(crate) entry_pos: dawn_core::AbsolutePosition,
    pub(crate) request_tick: Tick,
}

impl SimulationNode {
    pub(crate) fn transit_saga_snapshot(&self) -> TransitSagaSnapshot {
        self.transit.transit_journal.snapshot()
    }

    /// Return derived counts for active, retrying, terminal, and incoming
    /// Transit Saga records. The counters are diagnostic output, not recovery
    /// state separate from the Saga snapshot.
    pub fn transit_saga_diagnostics(&self) -> TransitSagaDiagnostics {
        self.transit.transit_journal.diagnostics()
    }

    pub(crate) fn restore_transit_saga(
        &mut self,
        snapshot: TransitSagaSnapshot,
    ) -> Result<(), String> {
        self.transit.transit_journal =
            crate::transit::handoff::TransitJournal::from_snapshot(self.sector_id, snapshot)?;
        let pending: Vec<_> = self
            .transit
            .transit_journal
            .pending_outgoing()
            .cloned()
            .collect();
        for attempt in pending {
            let Some(&entity) = self.simulation.ships.index.get(&attempt.ship_id) else {
                self.transit.transit_journal.quarantine(
                    attempt.attempt_id,
                    "outgoing Saga references a missing source Ship".to_owned(),
                );
                continue;
            };
            self.simulation
                .world
                .set_transit_state(entity, TransitState::InTransit { to: attempt.to });
        }
        Ok(())
    }

    pub(crate) fn register_incoming_transit_receipt(
        &mut self,
        receipt: IncomingTransitReceipt,
    ) -> bool {
        self.transit.transit_journal.register_incoming(receipt)
    }

    pub(crate) fn transit_attempt_acknowledged(&self, attempt_id: TransitAttemptId) -> bool {
        self.transit
            .transit_journal
            .outgoing(attempt_id)
            .is_some_and(|attempt| matches!(attempt.state, TransitAttemptState::Acknowledged))
    }

    pub(crate) fn quarantine_transit_attempt(
        &mut self,
        attempt_id: TransitAttemptId,
        reason: String,
    ) {
        self.transit.transit_journal.quarantine(attempt_id, reason);
    }

    fn allocate_transit_attempt(&mut self, ship_id: ShipId) -> Result<TransitAttemptId, DawnError> {
        loop {
            let sequence = self.transit.transit_attempt_counter;
            let next_sequence = sequence
                .checked_add(1)
                .ok_or(DawnError::TransitAttemptCounterOverflow(self.sector_id))?;
            self.transit.transit_attempt_counter = next_sequence;
            let attempt_id = TransitAttemptId::new(self.sector_id, ship_id, sequence);
            if self.transit.transit_journal.outgoing(attempt_id).is_none() {
                return Ok(attempt_id);
            }
        }
    }

    pub(crate) fn note_transit_commit_proposed(&mut self, attempt_id: TransitAttemptId) -> bool {
        self.transit
            .transit_journal
            .mark_commit_proposed(attempt_id, self.simulation.current_tick)
    }

    /// Validate and begin a Sector Transit (CLAUDE.md §4 Step 2).
    ///
    /// On success, marks the Ship `TransitState::InTransit` and appends a
    /// `SectorTransitRequested` event (ownership stays with this Sector).
    /// On failure, no event is appended (CommandRejected per INV-006).
    ///
    /// In the Raft pipeline (ADR-0014) this is invoked when a committed
    /// `TransitOp::Request` is applied at Step 7.5 — never directly from a
    /// client command. Folded into [`prepare_transit_commit`](Self::prepare_transit_commit);
    /// not called directly outside this module.
    #[cfg(test)]
    pub(in crate::node) fn propose_transit(
        &mut self,
        cmd: TransitCommand,
    ) -> Result<(), DawnError> {
        self.propose_transit_with_route(cmd, None, dawn_core::AbsolutePosition::ORIGIN)
            .map(|_| ())
    }

    fn begin_transit_with_route(
        &mut self,
        cmd: TransitCommand,
        gate_id: Option<JumpGateId>,
        entry_pos: dawn_core::AbsolutePosition,
    ) -> Result<(Tick, DomainEvent), DawnError> {
        let &entity = self
            .simulation
            .ships
            .index
            .get(&cmd.ship_id)
            .ok_or(DawnError::ShipNotFound(cmd.ship_id))?;

        if self.simulation.world.transit_state(entity).is_in_transit() {
            return Err(DawnError::ShipInTransit(cmd.ship_id));
        }

        self.simulation
            .world
            .set_transit_state(entity, TransitState::InTransit { to: cmd.to });
        let request_tick = self.simulation.current_tick;
        let event = DomainEvent::SectorTransitRequested(SectorTransitRequested {
            ship_id: cmd.ship_id,
            from: self.sector_id,
            to: cmd.to,
            request_tick,
            gate_id,
            entry_pos,
            tick: self.simulation.current_tick,
        });
        Ok((request_tick, event))
    }

    #[cfg(test)]
    fn propose_transit_with_route(
        &mut self,
        cmd: TransitCommand,
        gate_id: Option<JumpGateId>,
        entry_pos: dawn_core::AbsolutePosition,
    ) -> Result<Tick, DawnError> {
        let ship_id = cmd.ship_id;
        let (request_tick, event) = self.begin_transit_with_route(cmd, gate_id, entry_pos)?;
        let Some(handoff) = self.handoff_for_transit(ship_id) else {
            if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
                self.simulation
                    .world
                    .set_transit_state(entity, TransitState::None);
            }
            return Err(DawnError::ShipNotFound(ship_id));
        };
        let attempt_id = match self.allocate_transit_attempt(ship_id) {
            Ok(attempt_id) => attempt_id,
            Err(error) => {
                if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
                    self.simulation
                        .world
                        .set_transit_state(entity, TransitState::None);
                }
                return Err(error);
            }
        };
        self.transit
            .transit_journal
            .register_outgoing(OutgoingTransitAttempt {
                attempt_id,
                ship_id,
                from: self.sector_id,
                to: match &event {
                    DomainEvent::SectorTransitRequested(event) => event.to,
                    _ => unreachable!(),
                },
                handoff,
                gate_id,
                entry_pos,
                request_tick,
                state: TransitAttemptState::Prepared,
            });
        self.emit_event(event);
        Ok(request_tick)
    }

    pub(crate) fn is_ship_in_transit(&self, ship_id: ShipId) -> bool {
        self.simulation
            .ships
            .index
            .get(&ship_id)
            .is_some_and(|&entity| self.simulation.world.transit_state(entity).is_in_transit())
    }

    /// Whether a `TransitCommand` for `ship_id` would currently be accepted
    /// (Ship exists and is not already in transit). Used to reject commands
    /// up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_transit(&self, ship_id: ShipId) -> bool {
        self.simulation
            .ships
            .index
            .get(&ship_id)
            .is_some_and(|&entity| !self.simulation.world.transit_state(entity).is_in_transit())
    }

    #[cfg(test)]
    pub(crate) fn set_tackled_by_for_test(&mut self, ship_id: ShipId, tacklers: Vec<ShipId>) {
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return;
        };
        let _ = self
            .simulation
            .world
            .remove_one::<dawn_ecs::components::TackledComp>(entity);
        let _ = self
            .simulation
            .world
            .insert_one(entity, dawn_ecs::components::TackledComp { tacklers });
    }

    /// Stage 1 of a Sector Transit (ADR-0014 §3 [4]), as one action: validate
    /// and begin the Transit, work out where `ship_id` lands in `to` (a Jump
    /// Gate's `position`/`abs_m` leading back to this Sector, so the Ship can
    /// jump straight back — ADR-0009/0029 — or the Sector origin for a
    /// non-Gate Transit), and snapshot the Ship's state for the follow-up
    /// `TransitOp::Commit` proposal. Returns `None` if the Transit can't begin
    /// (unknown Ship, already in transit) or its canonical handoff snapshot
    /// cannot be built.
    pub(crate) fn prepare_transit_commit(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    ) -> Option<TransitCommitData> {
        let entry_pos = gate_id
            .and_then(|_| {
                self.galaxy()
                    .gates_in_sector(to)
                    .into_iter()
                    .find(|gate| gate.to_sector == self.sector_id())
            })
            .map(|gate| gate.abs_m)
            .unwrap_or(dawn_core::AbsolutePosition::ORIGIN);
        let (request_tick, event) = self
            .begin_transit_with_route(TransitCommand { ship_id, to }, gate_id, entry_pos)
            .ok()?;
        let Some(handoff) = self.handoff_for_transit(ship_id) else {
            if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
                self.simulation
                    .world
                    .set_transit_state(entity, TransitState::None);
            }
            return None;
        };
        let Ok(attempt_id) = self.allocate_transit_attempt(ship_id) else {
            if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
                self.simulation
                    .world
                    .set_transit_state(entity, TransitState::None);
            }
            return None;
        };
        self.transit
            .transit_journal
            .register_outgoing(OutgoingTransitAttempt {
                attempt_id,
                ship_id,
                from: self.sector_id,
                to,
                handoff: handoff.clone(),
                gate_id,
                entry_pos,
                request_tick,
                state: TransitAttemptState::Prepared,
            });
        self.emit_event(event);
        Some(TransitCommitData {
            attempt_id,
            handoff: Box::new(handoff),
            entry_pos,
            request_tick,
        })
    }

    /// Append `JumpGateUsed` (and `StarSystemChanged` if the destination
    /// Sector belongs to a different Star System) for a Ship that just
    /// completed a Jump-Gate Transit (ADR-0009).
    fn append_jump_events(
        &mut self,
        ship_id: ShipId,
        gate_id: JumpGateId,
        from: SectorId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
    ) {
        for event in self.jump_events(ship_id, gate_id, from, to, entry_pos) {
            self.emit_event(event);
        }
    }

    fn jump_events(
        &self,
        ship_id: ShipId,
        gate_id: JumpGateId,
        from: SectorId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
    ) -> Vec<DomainEvent> {
        let mut events = vec![DomainEvent::JumpGateUsed(JumpGateUsed {
            ship_id,
            gate_id,
            from_sector: from,
            to_sector: to,
            entry_pos,
            tick: self.simulation.current_tick,
        })];

        let from_system = self.topology.sector_map.galaxy.system_for_sector(from);
        let to_system = self.topology.sector_map.galaxy.system_for_sector(to);
        if from_system != to_system {
            events.push(DomainEvent::StarSystemChanged(StarSystemChanged {
                ship_id,
                from_system,
                to_system,
                tick: self.simulation.current_tick,
            }));
        }
        events
    }

    /// Read-only export for the `TransitOp::Commit` proposal.
    #[cfg(test)]
    pub(super) fn export_transit(&self, ship_id: ShipId) -> Option<TransitHandoffState> {
        self.handoff_for_transit(ship_id)
    }

    pub(crate) fn handoff_for_transit(&self, ship_id: ShipId) -> Option<TransitHandoffState> {
        let &entity = self.simulation.ships.index.get(&ship_id)?;
        if !self.simulation.world.transit_state(entity).is_in_transit() {
            return None;
        }
        let velocity = self.simulation.world.get::<VelocityComp>(entity)?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.simulation.world.get::<HullComp>(entity)?;
            (
                hull.shield(),
                hull.armor(),
                hull.hull(),
                hull.is_destroyed(),
            )
        };
        let capacitor = self
            .simulation
            .world
            .get::<CapacitorComp>(entity)
            .map(|c| c.current);
        let fitting = self
            .simulation
            .world
            .get::<FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(FittingSnapshot::empty);
        let ship_type_id = self
            .simulation
            .ships
            .type_ids
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipTypeId(0));
        let inventory = self
            .simulation
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();
        let owner_player_id = self.players.owners.get(&ship_id).copied();
        let (resume_ticket, pending_resume_ticket) = if owner_player_id.is_some() {
            self.client_resume_tickets(ship_id)
                .map(|(current, pending)| (Some(current), pending))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        Some(TransitHandoffState {
            ship_id,
            owner_player_id,
            resume_ticket,
            pending_resume_ticket,
            ship_type_id,
            velocity,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            inventory,
        })
    }

    /// Finalize the source half of a Sector Transit by removing the frozen
    /// recovery copy and appending `SectorTransitCompleted` to the source log.
    #[cfg(test)]
    pub(crate) fn complete_outgoing_transit(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        let Some(attempt_id) = self
            .transit
            .transit_journal
            .outgoing_for_ship(ship_id)
            .map(|attempt| attempt.attempt_id)
        else {
            return;
        };
        self.complete_outgoing_transit_for_attempt(attempt_id, to, entry_pos, request_tick);
    }

    pub(crate) fn complete_outgoing_transit_for_attempt(
        &mut self,
        attempt_id: TransitAttemptId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        let Some(handoff) = self
            .transit
            .transit_journal
            .outgoing(attempt_id)
            .map(|attempt| attempt.handoff.clone())
        else {
            return;
        };
        let Some(event) = self.complete_outgoing_state(&handoff, to, entry_pos, request_tick)
        else {
            self.transit.transit_journal.quarantine(
                attempt_id,
                "pending Transit source Ship is missing during Ack cleanup".to_owned(),
            );
            return;
        };
        self.emit_event(event);
        let _ = self.transit.transit_journal.mark_acknowledged(attempt_id);
    }

    fn complete_outgoing_state(
        &mut self,
        handoff: &TransitHandoffState,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) -> Option<DomainEvent> {
        if !self.simulation.ships.index.contains_key(&handoff.ship_id) {
            return None;
        }
        self.remove_ship(handoff.ship_id);
        Some(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                handoff: handoff.clone(),
                from: self.sector_id,
                to,
                request_tick,
                entry_pos,
                tick: self.simulation.current_tick,
            },
        ))
    }

    pub(crate) fn handle_transit_commit(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    ) {
        let ship_id = handoff.ship_id;
        self.import_transit(handoff, from, entry_pos, request_tick);
        if let Some(gate_id) = gate_id {
            let to = self.sector_id();
            self.append_jump_events(ship_id, gate_id, from, to, entry_pos);
        }
    }
}
