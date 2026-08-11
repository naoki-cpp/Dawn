//! Public Transit-event replay adapter.

#[cfg(test)]
use dawn_core::events::{SectorTransitAborted, SectorTransitCompleted, SectorTransitRequested};
#[cfg(test)]
use dawn_ecs::TransitState;

use super::super::SimulationNode;

impl SimulationNode {
    /// Tail replay of `SectorTransitRequested` (issue #204): mirrors
    /// `propose_transit`'s live effect on `TransitState`, so a Ship whose
    /// Transit was requested but not yet completed/aborted before a restart
    /// comes back marked `InTransit` instead of silently reverting to
    /// ordinary flight -- matching what the live node had.
    #[cfg(test)]
    pub(in crate::node) fn replay_sector_transit_requested(&mut self, e: &SectorTransitRequested) {
        if e.from == self.sector_id {
            if let Some(&entity) = self.simulation.ships.index.get(&e.ship_id) {
                self.simulation
                    .world
                    .set_transit_state(entity, TransitState::InTransit { to: e.to });
            }
        }

        if e.tick > self.simulation.current_tick {
            self.simulation.current_tick = e.tick;
        }
    }

    /// Tail replay of `SectorTransitAborted`: clears only the matching route's
    /// `InTransit` marker. `SectorTransitAborted` currently has no
    /// `request_tick`, so a same-route newer attempt cannot be distinguished;
    /// emitters must not append an Abort after superseding that route.
    #[cfg(test)]
    pub(in crate::node) fn replay_sector_transit_aborted(&mut self, e: &SectorTransitAborted) {
        if e.from == self.sector_id {
            if let Some(&entity) = self.simulation.ships.index.get(&e.ship_id) {
                if self.simulation.world.transit_state(entity)
                    == (TransitState::InTransit { to: e.to })
                {
                    self.simulation
                        .world
                        .set_transit_state(entity, TransitState::None);
                }
            }
        }
        if e.tick > self.simulation.current_tick {
            self.simulation.current_tick = e.tick;
        }
    }

    /// Tail replay of `SectorTransitCompleted` (issue #204).
    ///
    /// The destination uses the exact same absolute-arrival materialization
    /// seam as live Commit handling, but discards the events that are
    /// already present in the replayed log.
    #[cfg(test)]
    pub(in crate::node) fn replay_sector_transit_completed(&mut self, e: &SectorTransitCompleted) {
        if self.sector_id == e.from {
            self.remove_ship(e.handoff.ship_id);
        } else if self.sector_id == e.to {
            if self.simulation.ships.index.contains_key(&e.handoff.ship_id) {
                if e.tick > self.simulation.current_tick {
                    self.simulation.current_tick = e.tick;
                }
                return;
            }
            let _ = self.materialize_incoming_state(
                &e.handoff,
                e.from,
                e.entry_pos,
                e.request_tick,
                e.tick,
            );
        }
        if e.tick > self.simulation.current_tick {
            self.simulation.current_tick = e.tick;
        }
    }
}
