//! `SimulationNode` state transitions for Sector Transit (ADR-0014).
//!
//! The top-level `dawn-sector::transit` module owns the Raft payload
//! (`TransitOp`) and Step 7.5 orchestration. This module keeps the ECS and
//! EventStore mutations close to `SimulationNode`, where the required private
//! state already lives.

use dawn_core::{
    commands::TransitCommand,
    events::{JumpGateUsed, SectorTransitCompleted, SectorTransitRequested, StarSystemChanged},
    fitting::FittingSnapshot,
    DawnError, DomainEvent, JumpGateId, Position, SectorId, ShipId, ShipTypeId, Tick,
};
use dawn_ecs::{
    components::{CapacitorComp, FittingComp, HullComp, InventoryComp, PositionComp, VelocityComp},
    TransitState,
};
use dawn_event_store::store::EventStore;

use crate::persistence::ShipSnapshot;

use super::SimulationNode;

impl ShipSnapshot {
    /// The subset of this snapshot a Sector Transit carries across the
    /// Sector boundary (issue #204) -- see `TransitShipState`'s doc comment
    /// for why `position`/`anchor`/`velocity`/`tackled_by` are excluded.
    fn to_transit_ship_state(&self) -> dawn_core::events::TransitShipState {
        dawn_core::events::TransitShipState {
            ship_type_id: self.ship_type_id,
            current_shield: self.current_shield,
            current_armor: self.current_armor,
            current_hull: self.current_hull,
            is_destroyed: self.is_destroyed,
            capacitor: self.capacitor,
            fitting: self.fitting.clone(),
            inventory: self.inventory.clone(),
        }
    }
}

/// Rebuilds a `ShipSnapshot` from a `SectorTransitCompleted` event's
/// self-contained `ship_state`, for the destination Sector's tail replay
/// (`apply_event.rs`) to feed into `restore_ship_from_snapshot` the same way
/// `import_transit` does for the live path. `anchor` is the same placeholder
/// `snapshot_for_transit` uses -- overwritten by `rebase_after_transit`
/// right after restore, so its value here never survives past that call.
pub(super) fn ship_snapshot_from_transit(
    ship_id: dawn_core::ShipId,
    state: &dawn_core::events::TransitShipState,
    entry_pos: dawn_core::Position,
    velocity: dawn_core::Velocity,
) -> ShipSnapshot {
    ShipSnapshot {
        ship_id,
        ship_type_id: state.ship_type_id,
        absolute_position: None,
        position: entry_pos,
        anchor: dawn_core::AnchorId(0),
        velocity,
        current_shield: state.current_shield,
        current_armor: state.current_armor,
        current_hull: state.current_hull,
        is_destroyed: state.is_destroyed,
        capacitor: state.capacitor,
        fitting: state.fitting.clone(),
        tackled_by: Vec::new(),
        inventory: state.inventory.clone(),
    }
}

/// Everything the Raft layer needs to propose a `TransitOp::Commit`, produced
/// by [`SimulationNode::prepare_transit_commit`]. `ship` is boxed for the same
/// reason `TransitOp::Commit` boxes it (ADR-0032 grew `ShipSnapshot` with
/// `inventory`).
#[derive(Debug)]
pub struct TransitCommitData {
    pub ship: Box<ShipSnapshot>,
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

    fn propose_transit_with_route(
        &mut self,
        cmd: TransitCommand,
        gate_id: Option<JumpGateId>,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) -> Result<Tick, DawnError> {
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
        self.event_store.append(DomainEvent::SectorTransitRequested(
            SectorTransitRequested {
                ship_id: cmd.ship_id,
                from: self.sector_id,
                to: cmd.to,
                request_tick,
                gate_id,
                entry_pos,
                entry_pos_abs,
                tick: self.current_tick,
            },
        ));
        Ok(request_tick)
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
        let ship = self.snapshot_for_transit(ship_id)?;
        Some(TransitCommitData {
            ship: Box::new(ship),
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
    pub(super) fn append_jump_events(
        &mut self,
        ship_id: ShipId,
        gate_id: JumpGateId,
        from: SectorId,
        to: SectorId,
        entry_pos: dawn_core::AbsolutePosition,
    ) {
        self.event_store
            .append(DomainEvent::JumpGateUsed(JumpGateUsed {
                ship_id,
                gate_id,
                from_sector: from,
                to_sector: to,
                entry_pos,
                tick: self.current_tick,
            }));

        let from_system = self.sector_map.galaxy.system_for_sector(from);
        let to_system = self.sector_map.galaxy.system_for_sector(to);
        if from_system != to_system {
            self.event_store
                .append(DomainEvent::StarSystemChanged(StarSystemChanged {
                    ship_id,
                    from_system,
                    to_system,
                    tick: self.current_tick,
                }));
        }
    }

    /// Read-only export for the `TransitOp::Commit` proposal: snapshot the
    /// Ship's current state without removing it from this Sector's ECS or
    /// appending any event.
    ///
    /// Issue #204: this used to also remove the Ship and append
    /// `SectorTransitCompleted` here, at Request-commit time -- durably
    /// recording "the Ship left `from`" on this Sector's own log *before*
    /// the destination's `TransitOp::Commit` had even been proposed to Raft,
    /// let alone committed. A crash in that window left `from`'s log saying
    /// the Ship was gone while `to`'s log had nothing, so cluster-restart
    /// recovery could lose the Ship entirely -- worse than the resurrection
    /// bug replay recovery was fixing. The Ship now stays here, still
    /// `InTransit` (frozen out of Movement/Combat, `dawn-ecs`'s
    /// `TransitComp` guards), until [`complete_outgoing_transit`]
    /// (Self::complete_outgoing_transit) actually removes it -- which only
    /// runs once this same `TransitOp::Commit` is Raft-committed and this
    /// Sector observes its own echo of it (`transit::apply_committed_raft_entries`).
    /// Returns `None` if `ship_id` is unknown or not currently `InTransit`.
    /// Folded into [`prepare_transit_commit`](Self::prepare_transit_commit);
    /// not called directly outside this module.
    #[cfg(test)]
    pub(super) fn export_transit(&self, ship_id: ShipId) -> Option<ShipSnapshot> {
        self.snapshot_for_transit(ship_id)
    }

    fn snapshot_for_transit(&self, ship_id: ShipId) -> Option<ShipSnapshot> {
        let &entity = self.ships.index.get(&ship_id)?;
        if !self.world.transit_state(entity).is_in_transit() {
            return None;
        }

        let pos = self.world.get::<PositionComp>(entity)?.0;
        let vel = self.world.get::<VelocityComp>(entity)?.0;
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
        // Inventory must follow the ship across Sectors (ADR-0032) -- unlike
        // tackle, it's the pilot's possessions, not Sector-local state.
        let inventory = self
            .world
            .get::<InventoryComp>(entity)
            .map(|inv| inv.items.clone())
            .unwrap_or_default();

        // Tackle state is not transferred on sector transit (tacklers are in
        // this sector; they lose the tackle as the ship leaves).
        let snapshot = ShipSnapshot {
            ship_id,
            ship_type_id,
            absolute_position: None,
            position: pos,
            // Placeholder only: `AnchorId(0)` is a real, specific anchor
            // (Helios, Sector 0's star — see `anchor.rs`), not a "this
            // Sector's star" sentinel, so it is only ever correct by
            // coincidence for a transit out of Sector 0. `import_transit`'s
            // `rebase_after_transit` overwrites both this and `position`
            // with the real destination anchor before anything reads them
            // (ADR-0029), so the value here never survives past restore.
            anchor: dawn_core::AnchorId(0),
            velocity: vel,
            current_shield,
            current_armor,
            current_hull,
            is_destroyed,
            capacitor,
            fitting,
            tackled_by: Vec::new(),
            inventory,
        };

        Some(snapshot)
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

    /// Stage 1.5 of a Sector Transit (issue #204): actually removes the Ship
    /// from this (the `from`) Sector's ECS and appends `SectorTransitCompleted`
    /// from this Sector's perspective. Called only once this Sector observes
    /// its own `TransitOp::Commit` proposal get Raft-committed
    /// (`transit::apply_committed_raft_entries`'s `from == node.sector_id()`
    /// branch) — the durable removal is conditioned on the same fact the
    /// destination's import is conditioned on, so a crash before that Commit
    /// lands leaves the Ship exactly where `snapshot_for_transit` found it:
    /// still owned by `from`, still `InTransit`.
    ///
    /// Takes `ship` (the same `ShipSnapshot` the `TransitOp::Commit` payload
    /// carries, echoed back to this Sector along with everyone else's copy)
    /// rather than re-reading the Ship's current ECS state: the Ship has been
    /// frozen out of Movement/Combat since Request-commit
    /// (`dawn-ecs`'s `TransitComp` guards), so nothing should have changed it
    /// in the meantime, and using the one payload both `from` and `to` share
    /// keeps their `SectorTransitCompleted.ship_state` identical by
    /// construction instead of by coincidence.
    ///
    /// Idempotent: a no-op if the Ship is already gone (e.g. this Commit
    /// entry were ever observed twice).
    pub fn complete_outgoing_transit(
        &mut self,
        ship: &ShipSnapshot,
        to: SectorId,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) {
        if !self.ships.index.contains_key(&ship.ship_id) {
            return;
        }
        // remove_ship also clears owners/active_ship (ADR-0035 review: this used
        // to hand-roll index/type_ids/base_stats removal only, leaking a
        // dangling ownership entry for a transited player ship).
        self.remove_ship(ship.ship_id);

        self.event_store.append(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                ship_id: ship.ship_id,
                from: self.sector_id,
                to,
                entry_pos: entry_pos_abs,
                velocity: ship.velocity,
                tick: self.current_tick,
                ship_state: ship.to_transit_ship_state(),
            },
        ));
    }

    /// Complete an incoming Sector Transit: restore `ship` (exported from the
    /// `from` Sector via [`export_transit`](Self::export_transit)) into this
    /// node's ECS at `entry_pos`, preserving its `ShipId` (INV-004 — no ID
    /// reuse, the same Ship simply changes Sector ownership).
    ///
    /// `restore_ship_from_snapshot` re-applies the Ship's *old* (source-Sector)
    /// anchor, and `entry_pos` alone does not carry the destination anchor
    /// identity needed to use it as a raw offset. `entry_pos_abs` is the precise f64
    /// Sector-frame arrival point (the destination Gate's `abs_m`, or the
    /// origin for a non-Gate Transit); `rebase_after_transit` re-anchors
    /// against it (appending the authoritative `AnchorRebased` event, ADR-0029)
    /// so the Ship can immediately jump back out (ADR-0009).
    ///
    /// Appends `SectorTransitCompleted` from this (the `to`) Sector's
    /// perspective. Folded into [`handle_transit_commit`](Self::handle_transit_commit);
    /// not called directly outside this module.
    pub(super) fn import_transit(
        &mut self,
        ship: &ShipSnapshot,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
    ) {
        let mut ship = ship.clone();
        ship.position = entry_pos;
        self.restore_ship_from_snapshot(&ship);
        self.rebase_after_transit(ship.ship_id, entry_pos_abs);

        self.event_store.append(DomainEvent::SectorTransitCompleted(
            SectorTransitCompleted {
                ship_id: ship.ship_id,
                from,
                to: self.sector_id,
                entry_pos: entry_pos_abs,
                velocity: ship.velocity,
                tick: self.current_tick,
                ship_state: ship.to_transit_ship_state(),
            },
        ));
    }

    /// Stage 2 of a Sector Transit, as one action: import `ship` (from
    /// [`prepare_transit_commit`](Self::prepare_transit_commit) on the `from`
    /// Sector), then append `JumpGateUsed`/`StarSystemChanged` if this
    /// Transit came through a Jump Gate (ADR-0009). Callers no longer need to
    /// know that the Gate-event append is conditional and comes after import.
    ///
    /// The caller (`transit::apply_committed_raft_entries`) must already have
    /// checked `to == self.sector_id()` before calling — this only runs the
    /// import/event sequence, it doesn't re-check ownership of the Commit.
    pub fn handle_transit_commit(
        &mut self,
        ship: &ShipSnapshot,
        from: SectorId,
        entry_pos: Position,
        entry_pos_abs: dawn_core::AbsolutePosition,
        gate_id: Option<JumpGateId>,
    ) {
        let ship_id = ship.ship_id;
        self.import_transit(ship, from, entry_pos, entry_pos_abs);
        if let Some(gate_id) = gate_id {
            let to = self.sector_id();
            self.append_jump_events(ship_id, gate_id, from, to, entry_pos_abs);
        }
    }

    /// Re-anchor a Ship that just arrived in this Sector via Sector Transit
    /// to the nearest body anchor to `entry_pos_abs`, appending the
    /// authoritative `AnchorRebased` event (ADR-0029). No-op if the Ship or
    /// an anchor candidate in this Sector can't be found.
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
    ) {
        let Some((anchor, offset)) = self.rebase_ship_anchor_state(ship_id, entry_pos_abs) else {
            return;
        };
        self.event_store.append(DomainEvent::AnchorRebased(
            dawn_core::events::AnchorRebased {
                ship_id,
                anchor,
                offset,
                tick: self.current_tick,
            },
        ));
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
        if let Some(&entity) = self.ships.index.get(&e.ship_id) {
            self.world
                .set_transit_state(entity, dawn_ecs::TransitState::InTransit { to: e.to });
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
    /// The `to` branch does not call `restore_ship_from_snapshot` through
    /// `import_transit` (which also appends events) -- replay must not
    /// append anything it didn't already record, so it rebuilds a
    /// `ShipSnapshot` from `e.ship_state` via `ship_snapshot_from_transit`
    /// and redoes the anchor rebase state directly via
    /// `rebase_ship_anchor_state` (see that method's doc comment for why the
    /// already-logged `AnchorRebased` entry can't do this on its own).
    pub(super) fn replay_sector_transit_completed(
        &mut self,
        e: &dawn_core::events::SectorTransitCompleted,
    ) {
        if self.sector_id == e.from {
            self.remove_ship(e.ship_id);
        } else if self.sector_id == e.to && !self.ships.index.contains_key(&e.ship_id) {
            let entry_pos = Position::new(e.entry_pos[0], e.entry_pos[1], e.entry_pos[2]);
            let snapshot =
                ship_snapshot_from_transit(e.ship_id, &e.ship_state, entry_pos, e.velocity);
            self.restore_ship_from_snapshot(&snapshot);
            self.rebase_ship_anchor_state(e.ship_id, e.entry_pos);
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
    fn export_transit_snapshots_without_removing_the_ship_or_appending_an_event() {
        // Issue #204: export no longer removes the Ship or appends
        // SectorTransitCompleted -- that used to happen here, durably, before
        // the destination's TransitOp::Commit had even been proposed to Raft.
        // A crash in that window could lose the Ship (source's log said it
        // left, destination's log had nothing). Now the Ship stays put,
        // frozen (InTransit), until complete_outgoing_transit runs -- which
        // only happens once this Sector observes its own Commit land.
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
        node.complete_outgoing_transit(&snapshot, SectorId(1), entry_pos_abs);

        assert!(
            !node.ships.index.contains_key(&ship_id),
            "ship must leave the from-sector ECS"
        );
        assert_eq!(node.ship_count(), 0);

        let last = node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
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

        node.complete_outgoing_transit(&snapshot, SectorId(1), entry_pos_abs);
        node.complete_outgoing_transit(&snapshot, SectorId(1), entry_pos_abs);

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
            &snapshot,
            SectorId(1),
            dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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

        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos.into());

        assert_eq!(to_node.ship_count(), 1);
        assert_eq!(to_node.get_ship_position(ship_id), Some(entry_pos));

        let last = to_node.event_store().all_records().last().unwrap();
        match &last.event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.ship_id, ship_id);
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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos_abs);

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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
            &data.ship,
            SectorId(0),
            data.entry_pos,
            data.entry_pos_abs,
            Some(outbound_gate.id),
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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );

        for def in crate::modules::all_modules() {
            from_node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
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
        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos.into());

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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
        );

        // Before the handoff, the destination node rejects owned commands.
        assert!(!to_node.apply_stop_command_owned(player_id, ship_id));

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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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

        to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos.into());
        from_node.complete_outgoing_transit(&snapshot, SectorId(1), entry_pos.into());

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
                store,
            );
            to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos.into());

            let snap = to_node.take_snapshot();
            snap.save(&snap_path).unwrap();
        } // node drops; FileEventStore flushes via BufWriter

        let snap = StateSnapshot::load(&snap_path).unwrap();
        let store2 = FileEventStore::open(&event_path).unwrap();
        let restored = SimulationNode::restore_from(store2, &snap, &[], &[]);

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
            );
            let mut to_node = SimulationNode::new(
                NodeId(1),
                SectorId(1),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
            to_node.import_transit(&snapshot, SectorId(0), entry_pos, entry_pos.into());
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
                ship_id,
                from: SectorId(0), // matches node.sector_id() -- this is the source
                to: SectorId(1),
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                velocity: Velocity::ZERO,
                tick: Tick(1),
                ship_state: sample_transit_ship_state(),
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
        );
        let ship_id = ShipId::new(NodeId(0), 7);

        node.apply_event_pub(DomainEvent::SectorTransitCompleted(
            dawn_core::events::SectorTransitCompleted {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
                velocity: Velocity::new(1.0, 0.0, 0.0),
                tick: Tick(1),
                ship_state: sample_transit_ship_state(),
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
        );
        let ship_id = ShipId::new(NodeId(0), 7);
        let event =
            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                entry_pos: dawn_core::AbsolutePosition::new(500.0, 0.0, 0.0),
                velocity: Velocity::ZERO,
                tick: Tick(1),
                ship_state: sample_transit_ship_state(),
            });

        node.apply_event_pub(event.clone());
        node.apply_event_pub(event);

        assert_eq!(node.ship_count(), 1);
    }

    fn sample_transit_ship_state() -> dawn_core::events::TransitShipState {
        dawn_core::events::TransitShipState {
            ship_type_id: ShipTypeId(1),
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
        );
        let mut to_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
        to_node.import_transit(&exported, SectorId(0), entry_pos, entry_pos_abs);
        // The durability fix (issue #204) this test targets: `from_node` only
        // removes the ship and records SectorTransitCompleted once it
        // observes the *same* Commit the destination acted on -- mirroring
        // `transit::apply_committed_raft_entries`'s `from == node.sector_id()`
        // branch, not the old immediate removal at export time.
        from_node.complete_outgoing_transit(&exported, SectorId(1), entry_pos_abs);

        // Simulate a restart of both Sectors: snapshot + tail-log replay,
        // exactly as `restore_from` is used in production recovery.
        let mut from_store2 = InMemoryEventStore::new();
        for rec in from_node.event_store().all_records() {
            from_store2.append(rec.event.clone());
        }
        let restored_from =
            SimulationNode::restore_from(from_store2, &from_snapshot_before, &[], &[]);

        let mut to_store2 = InMemoryEventStore::new();
        for rec in to_node.event_store().all_records() {
            to_store2.append(rec.event.clone());
        }
        let restored_to = SimulationNode::restore_from(to_store2, &to_snapshot_before, &[], &[]);

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
        let restored = SimulationNode::restore_from(store2, &snapshot_before, &[], &[]);

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
