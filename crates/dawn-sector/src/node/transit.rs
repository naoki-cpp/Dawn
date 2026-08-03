//! Transit state mutation for `SimulationNode` (ADR-0014).
//!
//! This is the stateful half of the Transit deep module. The sibling
//! `crate::transit::pipeline` decides when a Request, Commit, or Ack should be proposed; this module
//! owns the Ship handoff lifecycle itself: freezing and snapshotting the
//! source, materializing and re-anchoring the destination, finalizing the
//! source after Ack, and applying the same facts during replay.
//!
//! The implementation remains an `impl` on `SimulationNode` because the ECS,
//! ownership maps, and EventStore are private node state. The module boundary
//! is nevertheless intentional: callers use Transit lifecycle operations and
//! do not reach into individual component mutations.

use dawn_core::{
    commands::TransitCommand,
    events::{JumpGateUsed, SectorTransitCompleted, SectorTransitRequested, StarSystemChanged},
    fitting::FittingSnapshot,
    DawnError, DomainEvent, JumpGateId, Position, SectorId, ShipId, ShipTypeId, Tick,
    TransitHandoffState,
};
use dawn_ecs::{
    components::{
        CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, ShipStatsComp,
        VelocityComp,
    },
    TransitState,
};
use dawn_event_store::store::EventStore;

use crate::persistence::CompletedIncomingTransit;

use super::SimulationNode;

/// Everything the Raft layer needs to propose a `TransitOp::Commit`, produced
/// by [`SimulationNode::prepare_transit_commit`].
#[derive(Debug)]
pub struct TransitCommitData {
    pub handoff: Box<TransitHandoffState>,
    pub entry_pos: Position,
    pub entry_pos_abs: dawn_core::AbsolutePosition,
    pub request_tick: Tick,
}

const TRANSIT_RETRY_INITIAL_TICKS: u64 = 10;
const TRANSIT_RETRY_MAX_TICKS: u64 = 160;

/// Process-local retry scheduling. The durable request and route live in the
/// EventStore; this component only prevents an unresolved request from being
/// proposed every Tick. It intentionally is not snapshotted, so restart causes
/// one immediate retry and then resumes bounded exponential backoff.
#[derive(Debug, Clone, Copy)]
struct TransitRetryComp {
    request_tick: Tick,
    next_retry_tick: Tick,
    backoff_ticks: u64,
}

impl<S: EventStore> SimulationNode<S> {
    pub(crate) fn has_completed_incoming_transit(
        &self,
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: Tick,
    ) -> bool {
        self.completed_incoming_transits
            .contains(&CompletedIncomingTransit {
                ship_id,
                from,
                to,
                request_tick,
            })
    }

    fn record_completed_incoming_transit(
        &mut self,
        ship_id: ShipId,
        from: SectorId,
        to: SectorId,
        request_tick: Tick,
    ) {
        self.completed_incoming_transits
            .push(CompletedIncomingTransit {
                ship_id,
                from,
                to,
                request_tick,
            });
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
    pub(super) fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {
        self.propose_transit_with_route(
            cmd,
            None,
            Position::ORIGIN,
            dawn_core::AbsolutePosition::ORIGIN,
        )
        .map(|_| ())
    }

    fn begin_transit_with_route(
        &mut self,
        cmd: TransitCommand,
        gate_id: Option<JumpGateId>,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) -> Result<(Tick, DomainEvent), DawnError> {
        let &entity = self
            .ships
            .index
            .get(&cmd.ship_id)
            .ok_or(DawnError::ShipNotFound(cmd.ship_id))?;

        if self.world.transit_state(entity).is_in_transit() {
            return Err(DawnError::ShipInTransit(cmd.ship_id));
        }

        self.world
            .set_transit_state(entity, TransitState::InTransit { to: cmd.to });
        let request_tick = self.current_tick;
        let event = DomainEvent::SectorTransitRequested(SectorTransitRequested {
            ship_id: cmd.ship_id,
            from: self.sector_id,
            to: cmd.to,
            request_tick,
            gate_id,
            entry_pos,
            entry_pos_abs,
            tick: self.current_tick,
        });
        Ok((request_tick, event))
    }

    fn propose_transit_with_route(
        &mut self,
        cmd: TransitCommand,
        gate_id: Option<JumpGateId>,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) -> Result<Tick, DawnError> {
        let (request_tick, event) =
            self.begin_transit_with_route(cmd, gate_id, entry_pos, entry_pos_abs)?;
        self.event_store.append(event);
        Ok(request_tick)
    }

    pub(crate) fn is_ship_in_transit(&self, ship_id: ShipId) -> bool {
        self.ships
            .index
            .get(&ship_id)
            .is_some_and(|&entity| self.world.transit_state(entity).is_in_transit())
    }

    /// Whether a `TransitCommand` for `ship_id` would currently be accepted
    /// (Ship exists and is not already in transit). Used to reject commands
    /// up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_transit(&self, ship_id: ShipId) -> bool {
        self.ships
            .index
            .get(&ship_id)
            .is_some_and(|&entity| !self.world.transit_state(entity).is_in_transit())
    }

    #[cfg(test)]
    pub(crate) fn set_tackled_by_for_test(&mut self, ship_id: ShipId, tacklers: Vec<ShipId>) {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let _ = self
            .world
            .remove_one::<dawn_ecs::components::TackledComp>(entity);
        let _ = self
            .world
            .insert_one(entity, dawn_ecs::components::TackledComp { tacklers });
    }

    /// Stage 1 of a Sector Transit (ADR-0014 §3 \[4\]), as one action: validate
    /// and begin the Transit, work out where `ship_id` lands in `to` (a Jump
    /// Gate's `position`/`abs_m` leading back to this Sector, so the Ship can
    /// jump straight back — ADR-0009/0029 — or the Sector origin for a
    /// non-Gate Transit), and snapshot the Ship's state for the follow-up
    /// `TransitOp::Commit` proposal. Returns `None` if the Transit can't begin
    /// (unknown Ship, already in transit) or the Ship can't be found to
    /// snapshot.
    ///
    /// Does **not** remove the Ship from this Sector's ECS (issue #204) --
    /// only `propose_transit`'s `TransitState::InTransit` marker changes here,
    /// which freezes the Ship out of Movement/Combat but keeps it durably
    /// owned by this Sector until [`Self::complete_outgoing_transit`] runs.
    ///
    /// Replaces the orchestrator in `transit::apply_committed_raft_entries`
    /// needing to know the Gate-lookup/entry-point logic itself — it now
    /// just wraps the result into a `TransitOp::Commit`.
    pub fn prepare_transit_commit(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        gate_id: Option<JumpGateId>,
    ) -> Option<TransitCommitData> {
        let arrival_gate = gate_id.and_then(|_| {
            self.galaxy()
                .gates_in_sector(to)
                .into_iter()
                .find(|gate| gate.to_sector == self.sector_id())
        });
        let entry_pos = arrival_gate
            .map(|gate| gate.position)
            .unwrap_or(Position::ORIGIN);
        let entry_pos_abs = arrival_gate
            .map(|gate| gate.abs_m)
            .unwrap_or(dawn_core::AbsolutePosition::ORIGIN);
        let request_tick = self
            .propose_transit_with_route(
                TransitCommand { ship_id, to },
                gate_id,
                entry_pos,
                entry_pos_abs,
            )
            .ok()?;
        let handoff = self.handoff_for_transit(ship_id)?;
        Some(TransitCommitData {
            handoff: Box::new(handoff),
            entry_pos,
            entry_pos_abs,
            request_tick,
        })
    }

    /// Append `JumpGateUsed` (and `StarSystemChanged` if the destination
    /// Sector belongs to a different Star System) for a Ship that just
    /// completed a Jump-Gate Transit (ADR-0009).
    ///
    /// Called from Step 7.5 on the destination node, after
    /// [`import_transit`](Self::import_transit) appends
    /// `SectorTransitCompleted` — `JumpGateUsed` records *how* the Ship
    /// moved, in addition to (not instead of) `SectorTransitCompleted`.
    /// Folded into [`handle_transit_commit`](Self::handle_transit_commit);
    /// not called directly outside this module.
    fn append_jump_events(
        &mut self,
        ship_id: ShipId,
        gate_id: JumpGateId,
        from: SectorId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
    ) {
        for event in self.jump_events(ship_id, gate_id, from, to, entry_pos) {
            self.event_store.append(event);
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
            tick: self.current_tick,
        })];

        let from_system = self.sector_map.galaxy.system_for_sector(from);
        let to_system = self.sector_map.galaxy.system_for_sector(to);
        if from_system != to_system {
            events.push(DomainEvent::StarSystemChanged(StarSystemChanged {
                ship_id,
                from_system,
                to_system,
                tick: self.current_tick,
            }));
        }
        events
    }

    /// Read-only export for the `TransitOp::Commit` proposal.
    #[cfg(test)]
    fn export_transit(&self, ship_id: ShipId) -> Option<TransitHandoffState> {
        self.handoff_for_transit(ship_id)
    }

    pub(crate) fn handoff_for_transit(&self, ship_id: ShipId) -> Option<TransitHandoffState> {
        let &entity = self.ships.index.get(&ship_id)?;
        if !self.world.transit_state(entity).is_in_transit() {
            return None;
        }
        let velocity = self.world.get::<VelocityComp>(entity)?.0;
        let (current_shield, current_armor, current_hull, is_destroyed) = {
            let hull = self.world.get::<HullComp>(entity)?;
            (
                hull.shield(),
                hull.armor(),
                hull.hull(),
                hull.is_destroyed(),
            )
        };
        let capacitor = self.world.get::<CapacitorComp>(entity).map(|c| c.current);
        let fitting = self
            .world
            .get::<FittingComp>(entity)
            .map(|f| f.to_snapshot())
            .unwrap_or_else(FittingSnapshot::empty);
        let ship_type_id = self
            .ships
            .type_ids
            .get(&ship_id)
            .copied()
            .unwrap_or(ShipTypeId(0));
        let inventory = self
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();
        Some(TransitHandoffState {
            ship_id,
            owner_player_id: self.ships.owners.get(&ship_id).copied(),
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

    pub(crate) fn transit_commit_retry_due(&self, ship_id: ShipId, request_tick: Tick) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        self.world
            .get::<TransitRetryComp>(entity)
            .map(|state| {
                state.request_tick != request_tick || self.current_tick >= state.next_retry_tick
            })
            .unwrap_or(true)
    }

    pub(crate) fn note_transit_commit_proposed(&mut self, ship_id: ShipId, request_tick: Tick) {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let existing = self
            .world
            .get::<TransitRetryComp>(entity)
            .map(|state| *state);
        let delay = existing
            .filter(|state| state.request_tick == request_tick)
            .map(|state| {
                state
                    .backoff_ticks
                    .saturating_mul(2)
                    .min(TRANSIT_RETRY_MAX_TICKS)
            })
            .unwrap_or(TRANSIT_RETRY_INITIAL_TICKS);
        let _ = self.world.remove_one::<TransitRetryComp>(entity);
        let _ = self.world.insert_one(
            entity,
            TransitRetryComp {
                request_tick,
                next_retry_tick: Tick(self.current_tick.value().saturating_add(delay)),
                backoff_ticks: delay,
            },
        );
    }

    /// Finalize the source half of a Sector Transit by removing the frozen
    /// recovery copy and appending `SectorTransitCompleted` to the source log.
    /// Normal completion calls this only after a committed Ack matches the
    /// durable Request identity. The superseded-outgoing path also uses it
    /// before accepting the same Ship back into this Sector. Until either
    /// condition is durable, the Ship remains owned by `from` and frozen as
    /// `InTransit`, so a crash cannot leave both logs without a recoverable copy.
    ///
    /// Ack carries only the transfer identity. The source re-reads the
    /// canonical handoff state from its frozen recovery copy before removal;
    /// `TransitComp` guards guarantee that state has not changed since Request.
    /// The resulting `SectorTransitCompleted.handoff` therefore matches the
    /// state previously proposed to the destination without coupling Ack to a
    /// second copy of the Ship payload.
    ///
    /// Idempotent: a no-op if the Ship is already gone (e.g. this Commit
    /// entry were ever observed twice).
    pub fn complete_outgoing_transit(
        &mut self,
        ship_id: ShipId,
        to: SectorId,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        let Some(handoff) = self.handoff_for_transit(ship_id) else {
            return;
        };
        let Some(event) = self.complete_outgoing_state(&handoff, to, entry_pos_abs, request_tick)
        else {
            return;
        };
        self.event_store.append(event);
    }

    fn complete_outgoing_state(
        &mut self,
        handoff: &TransitHandoffState,
        to: SectorId,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) -> Option<DomainEvent> {
        if !self.ships.index.contains_key(&handoff.ship_id) {
            return None;
        }
        self.remove_ship(handoff.ship_id);
        Some(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                handoff: handoff.clone(),
                from: self.sector_id,
                to,
                request_tick,
                entry_pos: entry_pos_abs,
                tick: self.current_tick,
            },
        ))
    }

    /// Complete an incoming Sector Transit from canonical handoff state.
    fn import_transit(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) {
        for event in
            self.materialize_incoming_state(handoff, from, entry_pos, entry_pos_abs, request_tick)
        {
            self.event_store.append(event);
        }
    }

    /// The single mapping from Transit handoff state into destination ECS.
    fn restore_ship_from_handoff(&mut self, handoff: &TransitHandoffState, entry_pos: Position) {
        self.insert_to_world(handoff.ship_id, entry_pos, handoff.velocity);
        self.ships
            .type_ids
            .insert(handoff.ship_id, handoff.ship_type_id);
        let base = self
            .ship_type_registry
            .get(&handoff.ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);
        self.base_stats.insert(handoff.ship_id, base);
        if let Some(&entity) = self.ships.index.get(&handoff.ship_id) {
            self.world.set_ship_stats(entity, base);
            let fitting = FittingComp::from_snapshot(&handoff.fitting, &self.module_registry);
            let _ = self.world.insert_one(entity, fitting);
            self.reapply_fitting(handoff.ship_id);
            if let Some(mut hull) = self.world.get_mut::<HullComp>(entity) {
                hull.set_hp(
                    handoff.current_shield,
                    handoff.current_armor,
                    handoff.current_hull,
                );
            }
            if let Some(current) = handoff.capacitor {
                let _ = self.world.insert_one(entity, CapacitorComp { current });
            }
            let _ = self.world.insert_one(
                entity,
                InventoryComp {
                    items: handoff.inventory.clone(),
                },
            );
        }
        if let Some(player_id) = handoff.owner_player_id {
            self.station_inventory_db
                .record_client_ownership(handoff.ship_id, player_id)
                .expect("transit owner binding transaction");
            debug_assert!(self.adopt_player_ship(handoff.ship_id, player_id));
        }
    }

    fn materialize_incoming_state(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        request_tick: Tick,
    ) -> Vec<DomainEvent> {
        self.restore_ship_from_handoff(handoff, entry_pos);
        let mut events = Vec::with_capacity(2);
        if let Some(event) = self.rebase_after_transit(handoff.ship_id, entry_pos_abs) {
            events.push(event);
        }
        events.push(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                handoff: handoff.clone(),
                from,
                to: self.sector_id,
                request_tick,
                entry_pos: entry_pos_abs,
                tick: self.current_tick,
            },
        ));
        events
    }

    pub fn handle_transit_commit(
        &mut self,
        handoff: &TransitHandoffState,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        gate_id: Option<JumpGateId>,
        request_tick: Tick,
    ) {
        let ship_id = handoff.ship_id;
        self.record_completed_incoming_transit(ship_id, from, self.sector_id, request_tick);
        self.import_transit(handoff, from, entry_pos, entry_pos_abs, request_tick);
        if let Some(gate_id) = gate_id {
            let to = self.sector_id();
            self.append_jump_events(ship_id, gate_id, from, to, entry_pos_abs);
        }
    }

    /// Re-anchor a Ship that just arrived in this Sector via Sector Transit
    /// to the nearest body anchor to `entry_pos_abs`, returning the
    /// authoritative `AnchorRebased` event for the caller to append
    /// (ADR-0029). Returns `None` if the Ship or an anchor candidate in this
    /// Sector can't be found.
    ///
    /// Unlike `warp::rebase_arrival_event` (which uses an all-zero
    /// `[f64; 3]` as an "arrival not engaged yet" sentinel), `entry_pos_abs`
    /// here is always a deliberate absolute point — a Gate's `abs_m`, or the
    /// Sector origin for a non-Gate Transit — so there's no fallback-compose
    /// branch to skip.
    fn rebase_after_transit(
        &mut self,
        ship_id: ShipId,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) -> Option<DomainEvent> {
        let (anchor, offset) = self.rebase_ship_anchor_state(ship_id, entry_pos_abs)?;
        Some(DomainEvent::AnchorRebased(
            dawn_core::events::AnchorRebased {
                ship_id,
                anchor,
                offset,
                tick: self.current_tick,
            },
        ))
    }

    /// The state-mutation half of `rebase_after_transit`, without appending
    /// `AnchorRebased`. Returns the `(anchor, offset)` that was set, or
    /// `None` if the Ship or an anchor candidate couldn't be found.
    ///
    /// Needed as its own step for tail replay of `SectorTransitCompleted`
    /// (issue #204): `import_transit` records `AnchorRebased` *before*
    /// `SectorTransitCompleted` in the log, so by the time a destination
    /// Sector's replay reaches the `AnchorRebased` entry the Ship doesn't
    /// exist there yet — that replay silently no-ops (`apply_event`'s
    /// `AnchorRebased` arm guards on the Ship already being present). The
    /// `SectorTransitCompleted` replay redoes this rebase itself, once the
    /// Ship exists, instead of appending a second `AnchorRebased`.
    fn rebase_ship_anchor_state(
        &mut self,
        ship_id: ShipId,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) -> Option<(dawn_core::AnchorId, Position)> {
        let &entity = self.ships.index.get(&ship_id)?;
        let to = self
            .anchor_table
            .nearest_anchor(self.sector_id, entry_pos_abs)?;
        let to_abs = self.anchor_table.abs(to)?;
        let offset = Position::new(
            entry_pos_abs[0] - to_abs[0],
            entry_pos_abs[1] - to_abs[1],
            entry_pos_abs[2] - to_abs[2],
        );
        self.world.set_ship_anchor(entity, to);
        if let Some(mut p) = self.world.get_mut::<PositionComp>(entity) {
            p.0 = offset;
        }
        Some((to, offset))
    }

    /// Tail replay of `SectorTransitRequested` (issue #204): mirrors
    /// `propose_transit`'s live effect on `TransitState`, so a Ship whose
    /// Transit was requested but not yet completed/aborted before a restart
    /// comes back marked `InTransit` instead of silently reverting to
    /// ordinary flight -- matching what the live node had.
    pub(super) fn replay_sector_transit_requested(
        &mut self,
        e: &dawn_core::events::SectorTransitRequested,
    ) {
        if e.from == self.sector_id {
            if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                self.world
                    .set_transit_state(entity, dawn_ecs::TransitState::InTransit { to: e.to });
            }
        }

        if e.tick > self.current_tick {
            self.current_tick = e.tick;
        }
    }

    /// Tail replay of `SectorTransitAborted`: clears the `InTransit` marker
    /// `SectorTransitRequested` replay set. Nothing in this codebase appends
    /// this event yet (its doc comment reserves it for a post-commit abort
    /// path not wired up today), but the replay side is written now rather
    /// than left a no-op, so the event type doesn't ship with a known-wrong
    /// replay the day something starts emitting it.
    pub(super) fn replay_sector_transit_aborted(
        &mut self,
        e: &dawn_core::events::SectorTransitAborted,
    ) {
        if let Some(&entity) = self.ships.index.get(&e.ship_id) {
            self.world
                .set_transit_state(entity, dawn_ecs::TransitState::None);
        }
        if e.tick > self.current_tick {
            self.current_tick = e.tick;
        }
    }

    /// Tail replay of `SectorTransitCompleted` (issue #204).
    ///
    /// `self.sector_id` decides which half of the live effect this Sector's
    /// log recorded: the `from` Sector removed the Ship
    /// (`complete_outgoing_transit`); the `to` Sector materialized it
    /// (`import_transit`). The two are mutually exclusive since a Transit
    /// always crosses Sectors (`from != to`).
    ///
    /// The `to` branch feeds `e.handoff` through the same direct
    /// handoff-to-ECS mapping as live import, without appending new events.
    /// It then redoes the anchor rebase state directly via
    /// `rebase_ship_anchor_state` (see that method's doc comment for why the
    /// already-logged `AnchorRebased` entry can't do this on its own).
    pub(super) fn replay_sector_transit_completed(
        &mut self,
        e: &dawn_core::events::SectorTransitCompleted,
    ) {
        if self.sector_id == e.from {
            self.remove_ship(e.handoff.ship_id);
        } else if self.sector_id == e.to {
            self.record_completed_incoming_transit(e.handoff.ship_id, e.from, e.to, e.request_tick);
            if self.ships.index.contains_key(&e.handoff.ship_id) {
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
                return;
            }
            let entry_pos = Position::new(e.entry_pos[0], e.entry_pos[1], e.entry_pos[2]);
            self.restore_ship_from_handoff(&e.handoff, entry_pos);
            self.rebase_ship_anchor_state(e.handoff.ship_id, e.entry_pos);
        }
        if e.tick > self.current_tick {
            self.current_tick = e.tick;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::StateSnapshot;
    use dawn_core::{NodeId, SectorBounds, Tick, Velocity};
    use dawn_event_store::FileEventStore;
    use dawn_event_store::InMemoryEventStore;

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn propose_transit_marks_ship_in_transit_and_appends_requested_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world.transit_state(entity),
            TransitState::InTransit { to: SectorId(1) }
        );

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitRequested(e) => {
                assert_eq!(e.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
            }
            other => panic!("expected SectorTransitRequested, got {other:?}"),
        }
    }

    #[test]
    fn propose_transit_is_rejected_when_ship_is_already_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let err = node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(2),
            })
            .unwrap_err();
        assert!(matches!(err, DawnError::ShipInTransit(id) if id == ship_id));
    }

    #[test]
    fn propose_transit_is_rejected_for_unknown_ship() {
        let mut node = mem_node();
        let unknown = ShipId::new(NodeId(99), 0);
        let err = node
            .propose_transit(TransitCommand {
                ship_id: unknown,
                to: SectorId(1),
            })
            .unwrap_err();
        assert!(matches!(err, DawnError::ShipNotFound(id) if id == unknown));
    }

    #[test]
    fn export_transit_handoff_without_removing_the_ship_or_appending_an_event() {
        // Issue #204: export no longer removes the Ship or appends
        // SectorTransitCompleted -- that used to happen here, durably, before
        // the destination's TransitOp::Commit had even been proposed to Raft.
        // A crash in that window could lose the Ship (source's log said it
        // left, destination's log had nothing). Now the Ship stays put,
        // frozen (InTransit), until a matching Ack is committed and
        // complete_outgoing_transit finalizes the source half.
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let snapshot = node.export_transit(ship_id).expect("ship should export");
        assert_eq!(snapshot.ship_id, ship_id);

        assert!(
            node.ships.index.contains_key(&ship_id),
            "the ship must stay in this Sector's ECS until complete_outgoing_transit"
        );
        assert!(
            !node
                .event_store()
                .all_records()
                .iter()
                .any(|r| matches!(r.event, DomainEvent::SectorTransitCompleted(_))),
            "export alone must not append SectorTransitCompleted"
        );
    }

    #[test]
    fn complete_outgoing_transit_removes_ship_and_appends_completed_event() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
        let snapshot = node.export_transit(ship_id).unwrap();

        let entry_pos_abs = dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0);
        node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos_abs, Tick::ZERO);

        assert!(
            !node.ships.index.contains_key(&ship_id),
            "ship must leave the from-sector ECS"
        );
        assert_eq!(node.ship_count(), 0);

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.handoff.ship_id, ship_id);
                assert_eq!(e.from, node.sector_id());
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos_abs);
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    #[test]
    fn complete_outgoing_transit_is_idempotent() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
        let snapshot = node.export_transit(ship_id).unwrap();
        let entry_pos_abs = dawn_core::AbsolutePosition::ORIGIN;

        node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos_abs, Tick::ZERO);
        node.complete_outgoing_transit(snapshot.ship_id, SectorId(1), entry_pos_abs, Tick::ZERO);

        let completed_count = node
            .event_store()
            .all_records()
            .iter()
            .filter(|r| matches!(r.event, DomainEvent::SectorTransitCompleted(_)))
            .count();
        assert_eq!(
            completed_count, 1,
            "a repeated Commit observation must not double-append"
        );
    }

    #[test]
    fn export_transit_clears_ownership_maps_for_a_player_ship() {
        // Regression test (architecture review 2026-07-03): export_transit
        // used to hand-roll index/type_ids/base_stats removal and forgot
        // owners/active_ship, leaving a dangling ownership entry for a
        // transited player ship. Now routed through ShipRegistry::remove
        // via SimulationNode::remove_ship (called by complete_outgoing_transit,
        // issue #204), which clears all four maps.
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();
        let snapshot = node.export_transit(ship_id).expect("ship should export");

        node.complete_outgoing_transit(
            snapshot.ship_id,
            SectorId(1),
            dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
            Tick::ZERO,
        );

        assert!(
            !node.owns_ship(player_id, ship_id),
            "owners map must not retain a dangling entry after transit"
        );
        assert!(
            !node.ships.owners.contains_key(&ship_id),
            "owners map must be cleared"
        );
        assert!(
            !node.ships.active_ship.contains_key(&player_id),
            "active_ship map must be cleared"
        );
    }

    #[test]
    fn export_transit_returns_none_for_ship_not_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.export_transit(ship_id).is_none());
        assert_eq!(node.ship_count(), 1, "ship must remain when not in transit");
    }

    #[test]
    fn import_transit_restores_ship_with_same_id_at_entry_position_and_appends_completed_event() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id).unwrap();

        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            entry_pos.into(),
            Tick::ZERO,
        );

        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));

        let last = to_node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.handoff.ship_id, ship_id);
                assert_eq!(e.from, SectorId(0));
                assert_eq!(e.to, SectorId(1));
                assert_eq!(e.entry_pos, entry_pos.into());
            }
            other => panic!("expected SectorTransitCompleted, got {other:?}"),
        }
    }

    /// Regression: a Ship that jumps through a Gate must land within the
    /// *return* Gate's `activation_radius`, so it can jump straight back.
    /// `entry_pos` alone is not sufficient to re-anchor against the destination
    /// body — `import_transit` must use
    /// the precise `entry_pos_abs` (the gate's `abs_m`) to set up the arriving
    /// Ship's anchor in the destination Sector (ADR-0029). Without that
    /// re-anchoring, the Ship keeps its *source*-Sector anchor and its
    /// absolute position computes to nonsense, so `can_propose_jump` for the
    /// return Gate (and every other Gate in the destination Sector) falsely
    /// fails.
    #[test]
    fn ship_arriving_through_a_gate_can_immediately_jump_back_through_the_return_gate() {
        let galaxy = crate::galaxy::Galaxy::demo();
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let return_gate = galaxy
            .gates_in_sector(SectorId(1))
            .into_iter()
            .find(|g| g.to_sector == SectorId(0))
            .expect("Sector 1 has a gate back to Sector 0");

        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();

        // Mirrors `transit::apply_committed_raft_entries`'s Request handler:
        // arrive at the return gate's position so the player can jump
        // straight back (ADR-0009).
        let entry_pos = return_gate.position;
        let entry_pos_abs = return_gate.abs_m;
        let snapshot = from_node.export_transit(ship_id).unwrap();
        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos_abs, Tick::ZERO);

        assert!(
            to_node.can_propose_jump(ship_id, return_gate.id),
            "ship must land within the return gate's activation_radius, not just \
             at its `position` interpreted against the wrong anchor"
        );
    }

    /// Same regression as above, but through the consolidated
    /// `prepare_transit_commit`/`handle_transit_commit` pair instead of the
    /// individual primitives — exercises the Gate-lookup/entry-point logic
    /// those two methods now own, mirroring exactly what
    /// `transit::apply_committed_raft_entries` calls in production.
    #[test]
    fn the_consolidated_request_commit_pair_reproduces_the_same_arrival() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let outbound_gate = crate::galaxy::Galaxy::demo()
            .gates_in_sector(SectorId(0))
            .into_iter()
            .find(|g| g.to_sector == SectorId(1))
            .expect("Sector 0 has a gate to Sector 1");
        let return_gate = crate::galaxy::Galaxy::demo()
            .gates_in_sector(SectorId(1))
            .into_iter()
            .find(|g| g.to_sector == SectorId(0))
            .expect("Sector 1 has a gate back to Sector 0");

        let ship_id = from_node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let data = from_node
            .prepare_transit_commit(ship_id, SectorId(1), Some(outbound_gate.id))
            .expect("transit must be accepted and the ship exported");
        assert_eq!(
            data.entry_pos_abs, return_gate.abs_m,
            "the arrival point must be the return gate's precise abs_m, not Sector 0's"
        );

        to_node.handle_transit_commit(
            &data.handoff,
            SectorId(0),
            data.entry_pos,
            data.entry_pos_abs,
            Some(outbound_gate.id),
            data.request_tick,
        );

        assert!(
            to_node.can_propose_jump(ship_id, return_gate.id),
            "the consolidated pair must reproduce the same anchor-fix as the primitives"
        );
        let records = to_node.event_store().all_records();
        let jump_used = records
            .iter()
            .find_map(|r| match &r.event {
                DomainEvent::JumpGateUsed(e) => Some(e),
                _ => None,
            })
            .expect("handle_transit_commit must append JumpGateUsed");
        assert_eq!(jump_used.ship_id, ship_id);
        assert_eq!(jump_used.gate_id, outbound_gate.id);
        assert!(
            records
                .iter()
                .any(|r| matches!(&r.event, DomainEvent::StarSystemChanged(_))),
            "Sector 0 (Alpha) and Sector 1 (Beta) are different Star Systems, \
             so StarSystemChanged must also be appended"
        );
    }

    #[test]
    fn inventory_survives_a_cross_sector_transit() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        for def in crate::game_data::test_catalog().modules().to_vec() {
            from_node.register_module(def);
        }
        for def in crate::game_data::test_catalog().ship_types().to_vec() {
            from_node.register_ship_type(def);
        }
        let player_id = from_node.next_player_id();
        let ship_id = from_node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let before_entity = *from_node.ships.index.get(&ship_id).unwrap();
        let before_len = from_node
            .world
            .get::<dawn_ecs::components::InventoryComp>(before_entity)
            .unwrap()
            .items
            .values()
            .copied()
            .sum::<u64>();
        assert!(before_len > 0, "player ships spawn with a seeded inventory");

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id).unwrap();
        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            entry_pos.into(),
            Tick::ZERO,
        );

        let after_entity = *to_node.ships.index.get(&ship_id).unwrap();
        let after = to_node
            .world
            .get::<dawn_ecs::components::InventoryComp>(after_entity)
            .unwrap();
        assert_eq!(
            after.items.values().copied().sum::<u64>(),
            before_len,
            "inventory must carry over the gate, unlike tackle state"
        );
    }

    #[test]
    fn adopted_player_ship_accepts_owned_commands_on_the_destination_node() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let player_id = from_node.next_player_id();
        let ship_id = from_node.spawn_player_ship(player_id);
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let snapshot = from_node.export_transit(ship_id).unwrap();
        to_node.import_transit(
            &snapshot,
            SectorId(0),
            Position::ORIGIN,
            dawn_core::AbsolutePosition::ORIGIN,
            Tick::ZERO,
        );

        // The durable handoff establishes ownership before any client resume.
        assert!(to_node.apply_stop_command_owned(player_id, ship_id));

        assert!(to_node.adopt_player_ship(ship_id, player_id));
        assert!(to_node.apply_stop_command_owned(player_id, ship_id));
    }

    /// Normal-path Sector Transit: ownership ends up in exactly one Sector,
    /// and at no point do both Sectors hold the Ship at once (INV-003).
    #[test]
    fn transit_moves_ship_ownership_to_destination_sector_exactly_once() {
        // Issue #204 strengthened this invariant: ownership now stays with
        // exactly one Sector for the *entire* Transit, never dropping to zero
        // in between. Before, `export_transit` removed the ship immediately
        // at Request-commit time, so there was a real window (until the
        // destination's Commit landed) where the sum below was 0 -- which is
        // exactly the crash-loses-the-ship window this issue closed. Now the
        // source keeps the ship (frozen out of Movement/Combat) until it
        // observes its own Commit, so the sum is always 1.
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        assert_eq!(
            from_node.ship_count() + to_node.ship_count(),
            1,
            "ship starts owned by exactly one sector"
        );

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        // Proposal alone does not move ownership yet.
        assert_eq!(from_node.ship_count() + to_node.ship_count(), 1);

        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id).unwrap();

        // Exporting a snapshot for the Commit proposal does not move
        // ownership either -- the ship is still durably owned by `from`,
        // just frozen (`TransitState::InTransit`) until the Commit lands.
        assert_eq!(
            from_node.ship_count() + to_node.ship_count(),
            1,
            "export must not create a window where neither Sector owns the ship"
        );
        assert_eq!(from_node.ship_count(), 1);
        assert_eq!(to_node.ship_count(), 0);

        to_node.import_transit(
            &snapshot,
            SectorId(0),
            entry_pos,
            entry_pos.into(),
            Tick::ZERO,
        );
        from_node.complete_outgoing_transit(
            snapshot.ship_id,
            SectorId(1),
            entry_pos.into(),
            Tick::ZERO,
        );

        // Final state: destination sector owns the ship, exactly once overall.
        assert_eq!(from_node.ship_count(), 0);
        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));
    }

    /// INV-002: after a Sector Transit, the destination Sector's state can be
    /// fully reproduced from a snapshot + Event Log replay (node restart).
    #[test]
    fn destination_sector_state_after_transit_is_fully_restored_from_snapshot_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let event_path = dir.path().join("events.log");
        let snap_path = dir.path().join("snapshot.bin");

        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let snapshot = from_node.export_transit(ship_id).unwrap();

        {
            let store = FileEventStore::open(&event_path).unwrap();
            let mut to_node = SimulationNode::with_store(
                NodeId(1),
                SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
                store,
            );
            to_node.import_transit(
                &snapshot,
                SectorId(0),
                entry_pos,
                entry_pos.into(),
                Tick::ZERO,
            );

            let snap = to_node.take_snapshot();
            snap.save(&snap_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap = StateSnapshot::load(&snap_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let restored = SimulationNode::restore_from(
            store2,
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );

        assert_eq!(restored.ship_count(), 1);
        assert_eq!(restored.get_ship_position(ship_id), Some(entry_pos));
    }

    /// ADR-0014 Task 9: measures the cost of a single Sector Transit
    /// (propose + export + import), excluding Raft commit latency.
    ///
    /// Ignored by default (it's a benchmark, not a correctness check).
    /// Run with: `cargo test -p dawn-simulation --release transit_latency_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn transit_latency_benchmark() {
        use std::time::Instant;

        const ITERATIONS: u32 = 1_000;
        let mut total = std::time::Duration::ZERO;

        for i in 0..ITERATIONS {
            let mut from_node = SimulationNode::new(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            );
            let mut to_node = SimulationNode::new(
                NodeId(1),
                SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            );
            let ship_id = from_node.spawn_ship(
                ShipTypeId(1),
                Position::ORIGIN,
                Velocity::new(1.0, 0.0, 0.0),
            );
            let entry_pos = Position::new(500.0, 0.0, 0.0);

            let start = Instant::now();
            from_node
                .propose_transit(TransitCommand {
                    ship_id,
                    to: SectorId(1),
                })
                .unwrap();
            let snapshot = from_node.export_transit(ship_id).unwrap();
            to_node.import_transit(
                &snapshot,
                SectorId(0),
                entry_pos,
                entry_pos.into(),
                Tick::ZERO,
            );
            total += start.elapsed();

            let _ = i;
        }

        let avg = total / ITERATIONS;
        println!("transit (propose+export+import) avg over {ITERATIONS} iterations: {avg:?}");
    }

    // ── Snapshot + tail replay recovery (issue #204) ────────────────────────

    #[test]
    fn replaying_requested_marks_the_ship_in_transit() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.apply_event_pub(DomainEvent::SectorTransitRequested(
            dawn_core::events::SectorTransitRequested {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                request_tick: Tick(1),
                gate_id: None,
                entry_pos: dawn_core::Position::ORIGIN,
                entry_pos_abs: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(1),
            },
        ));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world.transit_state(entity),
            TransitState::InTransit { to: SectorId(1) }
        );
    }

    #[test]
    fn replaying_an_incoming_request_marker_does_not_freeze_the_destination_ship() {
        let mut node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_event_pub(DomainEvent::SectorTransitRequested(
            dawn_core::events::SectorTransitRequested {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                request_tick: Tick(7),
                gate_id: None,
                entry_pos: Position::ORIGIN,
                entry_pos_abs: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(3),
            },
        ));

        assert!(node.can_propose_transit(ship_id));
    }

    #[test]
    fn replaying_aborted_clears_the_in_transit_marker() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_event_pub(DomainEvent::SectorTransitRequested(
            dawn_core::events::SectorTransitRequested {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                request_tick: Tick(1),
                gate_id: None,
                entry_pos: dawn_core::Position::ORIGIN,
                entry_pos_abs: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(1),
            },
        ));

        node.apply_event_pub(DomainEvent::SectorTransitAborted(
            dawn_core::events::SectorTransitAborted {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                tick: Tick(2),
            },
        ));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(node.world.transit_state(entity), TransitState::None);
    }

    #[test]
    fn replaying_completed_on_the_source_sector_removes_the_ship() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.apply_event_pub(DomainEvent::SectorTransitCompleted(
            dawn_core::events::SectorTransitCompleted {
                handoff: sample_transit_handoff(ship_id, Velocity::ZERO),
                from: SectorId(0), // matches node.sector_id() -- this is the source
                to: SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(1),
            },
        ));

        assert!(
            !node.ships.index.contains_key(&ship_id),
            "a Sector replaying its own SectorTransitCompleted as the source \
             must not resurrect the ship it exported"
        );
    }

    #[test]
    fn replaying_completed_on_the_destination_sector_materializes_the_ship() {
        let mut node = SimulationNode::new(
            NodeId(1),
            SectorId(1), // matches `to` below -- this is the destination
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let ship_id = ShipId::new(NodeId(0), 7);

        node.apply_event_pub(DomainEvent::SectorTransitCompleted(
            dawn_core::events::SectorTransitCompleted {
                handoff: sample_transit_handoff(ship_id, Velocity::new(1.0, 0.0, 0.0)),
                from: SectorId(0),
                to: SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
                tick: Tick(1),
            },
        ));

        assert!(
            node.ships.index.contains_key(&ship_id),
            "a Sector replaying SectorTransitCompleted as the destination \
             must materialize the imported ship from the event alone"
        );
        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world.get::<VelocityComp>(entity).unwrap().0,
            Velocity::new(1.0, 0.0, 0.0)
        );
    }

    #[test]
    fn replaying_completed_on_the_destination_sector_is_idempotent() {
        // Guards against a double-materialize if the event were ever
        // replayed twice (e.g. a snapshot taken mid-tail-replay in a future
        // refactor) -- mirrors the `!contains_key` guard every other
        // ship-materializing replay arm already has (ShipSpawned, etc).
        let mut node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let ship_id = ShipId::new(NodeId(0), 7);
        let event =
            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                handoff: sample_transit_handoff(ship_id, Velocity::ZERO),
                from: SectorId(0),
                to: SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
                tick: Tick(1),
            });

        node.apply_event_pub(event.clone());
        node.apply_event_pub(event);

        assert_eq!(node.ship_count(), 1);
    }

    fn sample_transit_handoff(ship_id: ShipId, velocity: Velocity) -> TransitHandoffState {
        TransitHandoffState {
            ship_id,
            owner_player_id: None,
            ship_type_id: ShipTypeId(1),
            velocity,
            current_shield: 80.0,
            current_armor: 90.0,
            current_hull: 100.0,
            is_destroyed: false,
            capacitor: Some(40.0),
            fitting: dawn_core::fitting::FittingSnapshot::empty(),
            inventory: std::collections::BTreeMap::new(),
        }
    }

    /// The end-to-end acceptance test from issue #204: a completed
    /// cross-Sector Transit must survive a simulated restart of *both*
    /// Sectors, and ownership must land on exactly one of them afterward --
    /// never both (a resurrected source ship) and never neither (a lost
    /// import).
    #[test]
    fn a_completed_transit_survives_snapshot_plus_tail_replay_on_both_sectors() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        // Snapshot both Sectors *before* the transit -- the ship only exists
        // in the post-snapshot tail, on both sides, the same shape as issue
        // #197's regression.
        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        let from_snapshot_before = from_node.take_snapshot();
        let to_snapshot_before = to_node.take_snapshot();

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        let entry_pos = Position::new(500.0, 0.0, 0.0);
        let entry_pos_abs = dawn_core::AbsolutePosition::from(entry_pos);
        let exported = from_node.export_transit(ship_id).unwrap();
        to_node.import_transit(&exported, SectorId(0), entry_pos, entry_pos_abs, Tick::ZERO);
        // The durability fix (issue #204) this test targets: `from_node` only
        // removes the ship and records SectorTransitCompleted once it
        // observes the *same* Commit the destination acted on -- mirroring
        // `transit::apply_committed_raft_entries`'s `from == node.sector_id()`
        // branch, not the old immediate removal at export time.
        from_node.complete_outgoing_transit(
            exported.ship_id,
            SectorId(1),
            entry_pos_abs,
            Tick::ZERO,
        );

        // Simulate a restart of both Sectors: snapshot + tail-log replay,
        // exactly as `restore_from` is used in production recovery.
        let mut from_store2 = InMemoryEventStore::new();
        for rec in from_node.event_store().all_records() {
            from_store2.append(rec.event.clone());
        }
        let restored_from = SimulationNode::restore_from(
            from_store2,
            &from_snapshot_before,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );

        let mut to_store2 = InMemoryEventStore::new();
        for rec in to_node.event_store().all_records() {
            to_store2.append(rec.event.clone());
        }
        let restored_to = SimulationNode::restore_from(
            to_store2,
            &to_snapshot_before,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );

        let owned_by_source = restored_from.ships.index.contains_key(&ship_id);
        let owned_by_destination = restored_to.ships.index.contains_key(&ship_id);
        assert!(
            !owned_by_source,
            "the source Sector must not resurrect a ship it transferred away"
        );
        assert!(
            owned_by_destination,
            "the destination Sector must reconstruct the imported ship from \
             its own tail log alone"
        );
        assert_ne!(
            owned_by_source, owned_by_destination,
            "ownership must exist on exactly one Sector after restart, never \
             both and never neither"
        );

        let entity = *restored_to.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            restored_to.world.get::<VelocityComp>(entity).unwrap().0,
            Velocity::new(1.0, 0.0, 0.0),
            "velocity"
        );
        assert_eq!(
            restored_to.get_ship_position(ship_id),
            Some(entry_pos),
            "the imported ship must land at the transit entry position, \
             the same as the live import_transit path"
        );
        // The load-bearing check: the destination replay must redo the
        // anchor rebase itself (rebase_ship_anchor_state), not rely on the
        // already-logged AnchorRebased entry -- that entry replays *before*
        // the ship exists on this Sector (see that method's doc comment) and
        // silently no-ops. Comparing against the live `to_node`'s anchor
        // (produced by the same import through the normal, working path)
        // catches a dropped rebase that a position-only check cannot: with no
        // nearby body to rebase onto, this Sector's `AnchorTable` falls back
        // to the same default anchor either way, so position alone reads
        // identical whether or not the rebase ran. Anchor identity does not.
        assert_eq!(
            restored_to.get_ship_anchor(ship_id),
            to_node.get_ship_anchor(ship_id),
            "the restored ship's anchor must match what the live import produced"
        );
    }

    /// The crash window a review of the first version of this fix caught
    /// (issue #204): a cluster restart between the source's `TransitOp::Request`
    /// commit and the destination's `TransitOp::Commit` commit must not lose
    /// the ship. Before deferring `complete_outgoing_transit` to Commit-time,
    /// `export_transit` removed the ship and appended `SectorTransitCompleted`
    /// immediately at Request-commit time -- durably, on the source's own
    /// log -- before the destination's Commit had even been *proposed* to
    /// Raft, let alone committed. A restart in that gap left the source log
    /// saying the ship was gone and the destination log with nothing at all:
    /// the ship existed nowhere. This test stops at exactly that point --
    /// `export_transit` runs (building the Commit proposal payload) but
    /// neither `complete_outgoing_transit` nor `import_transit` ever does --
    /// and asserts the source Sector still owns the ship after a simulated
    /// restart.
    #[test]
    fn a_ship_survives_a_restart_between_request_commit_and_transit_commit() {
        let mut from_node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        );

        let ship_id = from_node.spawn_ship(
            ShipTypeId(1),
            Position::ORIGIN,
            Velocity::new(1.0, 0.0, 0.0),
        );
        let snapshot_before = from_node.take_snapshot();

        from_node
            .propose_transit(TransitCommand {
                ship_id,
                to: SectorId(1),
            })
            .unwrap();
        // Mirrors `prepare_transit_commit`'s snapshot step, run for a
        // `TransitOp::Commit` that -- in this test -- is never proposed,
        // never mind committed. Nothing past this point ever runs:
        // no `complete_outgoing_transit`, no destination `import_transit`.
        let _snapshot_for_commit_proposal = from_node.export_transit(ship_id).unwrap();

        // Simulate a whole-cluster restart at exactly this point.
        let mut store2 = InMemoryEventStore::new();
        for rec in from_node.event_store().all_records() {
            store2.append(rec.event.clone());
        }
        let restored = SimulationNode::restore_from(
            store2,
            &snapshot_before,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &[],
            &[],
        );

        assert!(
            restored.ships.index.contains_key(&ship_id),
            "a restart before the Commit lands must not lose the ship -- it \
             is still owned by the source Sector, just pending"
        );
        let entity = *restored.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            restored.world.transit_state(entity),
            TransitState::InTransit { to: SectorId(1) },
            "the ship must still be marked InTransit, so it stays frozen \
             (Movement/Combat) and a retried Commit is still meaningful"
        );
    }
}
