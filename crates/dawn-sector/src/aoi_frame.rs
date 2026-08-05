//! Deep Area-of-Interest frame lifecycle (ADR-0019).
//!
//! The spatial index is derived and non-persistent. This module deliberately
//! rebuilds it from authoritative ship positions at each delivery frame, then
//! resolves each observer's 27-cell visible set and delegates the ordered
//! Enter/Leave/event/correction delivery to [`crate::aoi::AoiDelivery`].
//! Runtime crates provide only session sinks and Sector routing; they do not
//! construct grids or visible sets themselves.

use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_event_store::store::EventStore;

use crate::aoi::{AoiDelivery, AoiSink, CellGrid, Observer};
use crate::node::SimulationNode;

/// One Sector's complete AoI delivery frame state.
///
/// The index policy is **rebuild per delivery frame**, not incremental update.
/// Rebuilding keeps the index purely derived from authoritative positions,
/// gives recovery the same path as normal execution, and preserves deterministic
/// `ShipId`-sorted enumeration without runtime-specific lifecycle code.
#[derive(Debug)]
pub struct AoiFrame {
    cell_size: f64,
    index: CellGrid,
    delivery: AoiDelivery,
}

impl AoiFrame {
    /// Create an empty frame owner. Call [`Self::rebuild`] before ordinary
    /// delivery, or [`Self::seed_observer`] when admitting/resuming a session.
    pub fn new(cell_size: f64) -> Self {
        Self {
            cell_size,
            index: CellGrid::new(cell_size),
            delivery: AoiDelivery::new(),
        }
    }

    /// Reconstruct the derived spatial index from the node's authoritative
    /// current positions. Every runtime calls this once after its simulation
    /// Tick and before delivering that Tick's frame.
    pub fn rebuild<S: EventStore>(&mut self, node: &SimulationNode<S>) {
        self.index = CellGrid::build(self.cell_size, node.ship_absolute_positions());
    }

    /// Rebuild from authoritative state and seed one observer without emitting
    /// Enter/Leave. This is the admission, redirect/resume, and recovery-safe
    /// entry point: delivery never depends on a persisted or runtime-owned grid.
    pub fn seed_observer<S: EventStore>(&mut self, node: &SimulationNode<S>, observer: Observer) {
        self.rebuild(node);
        self.seed_observer_from_index(node, observer);
    }

    /// Seed from the index already rebuilt for the current frame. Cluster
    /// handoff uses this after all Sector indexes have been reconstructed.
    pub fn seed_observer_from_index<S: EventStore>(
        &mut self,
        node: &SimulationNode<S>,
        observer: Observer,
    ) {
        let visible = self.visible_for(node, observer.ship_id);
        self.delivery.seed_player(observer.player_id, visible);
    }

    /// Resolve the observer from this frame's index, compute visible-set
    /// changes, and emit the complete ordered AoI frame.
    pub fn deliver_observer<S: EventStore>(
        &mut self,
        sink: &mut dyn AoiSink,
        node: &SimulationNode<S>,
        observer: Observer,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let visible = self.visible_for(node, observer.ship_id);
        self.delivery
            .deliver_frame(sink, node, observer, visible, new_events, warp_arrivals)
    }

    /// Drop per-player visible-set memory for sessions no longer owned by this
    /// Sector frame.
    pub fn retain_players(&mut self, keep: impl Fn(PlayerId) -> bool) {
        self.delivery.retain_players(keep);
    }

    fn visible_for<S: EventStore>(&self, node: &SimulationNode<S>, ship_id: ShipId) -> Vec<ShipId> {
        node.ship_absolute_pos(ship_id)
            .map(|position| self.index.neighbors_of(position))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId, Velocity};
    use dawn_wire::ServerMessage;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct FakeSink {
        enters: Vec<u64>,
        leaves: Vec<u64>,
    }

    impl AoiSink for FakeSink {
        fn send_events(&mut self, _events: &[DomainEvent]) -> bool {
            true
        }

        fn send_message(&mut self, message: &ServerMessage) -> bool {
            match message {
                ServerMessage::AoiEnter(ship) => self.enters.push(ship.ship_id),
                ServerMessage::AoiLeave { ship_id } => self.leaves.push(*ship_id),
                _ => {}
            }
            true
        }
    }

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn admission_rebuilds_the_index_from_authoritative_state_before_seeding() {
        let mut node = node();
        let own = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let other = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let observer = Observer {
            player_id: PlayerId(1),
            ship_id: own,
        };
        let mut frame = AoiFrame::new(100.0);

        assert!(frame.visible_for(&node, own).is_empty());
        frame.seed_observer(&node, observer);
        assert_eq!(frame.visible_for(&node, own), vec![own, other]);

        let mut sink = FakeSink::default();
        assert!(frame.deliver_observer(
            &mut sink,
            &node,
            Observer {
                player_id: PlayerId(1),
                ship_id: own,
            },
            &[],
            &[],
        ));
        assert!(sink.enters.is_empty());
        assert!(sink.leaves.is_empty());
    }

    #[test]
    fn rebuilding_replaces_the_index_instead_of_incrementally_retaining_stale_ships() {
        let mut node = node();
        let own = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let mut frame = AoiFrame::new(100.0);
        frame.rebuild(&node);
        assert_eq!(frame.visible_for(&node, own), vec![own]);

        let other = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        frame.rebuild(&node);
        assert_eq!(frame.visible_for(&node, own), vec![own, other]);
    }
}
