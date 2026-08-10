//! Deep Area-of-Interest frame lifecycle (ADR-0019).
//!
//! The spatial index is derived and non-persistent. This module deliberately
//! rebuilds it from authoritative ship positions at each delivery frame, then
//! resolves each observer's 27-cell visible set and delegates the ordered
//! Enter/Leave/event/correction delivery to [`crate::aoi::AoiDelivery`].
//! Runtime crates provide only session sinks and Sector routing; they do not
//! construct grids or visible sets themselves.

use dawn_core::{DomainEvent, PlayerId, SectorId, ShipId};

use crate::aoi::{AoiDelivery, AoiSink, CellGrid, Observer};
use crate::view::SectorView;
use std::collections::{HashMap, HashSet};

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
    pub fn rebuild<V: SectorView>(&mut self, view: &V) {
        self.index = CellGrid::build(self.cell_size, view.ship_absolute_positions());
    }

    /// Rebuild from authoritative state and seed one observer without emitting
    /// Enter/Leave. This is the admission, redirect/resume, and recovery-safe
    /// entry point: delivery never depends on a persisted or runtime-owned grid.
    pub fn seed_observer<V: SectorView>(&mut self, view: &V, observer: Observer) {
        self.rebuild(view);
        self.seed_observer_from_index(view, observer);
    }

    /// Seed from the index already rebuilt for the current frame. Cluster
    /// handoff uses this after all Sector indexes have been reconstructed.
    pub fn seed_observer_from_index<V: SectorView>(&mut self, view: &V, observer: Observer) {
        let visible = self.visible_for(view, observer.ship_id);
        self.delivery.seed_player(observer.player_id, visible);
    }

    /// Resolve the observer from this frame's index, compute visible-set
    /// changes, and emit the complete ordered AoI frame.
    pub fn deliver_observer<V: SectorView>(
        &mut self,
        sink: &mut dyn AoiSink,
        view: &V,
        observer: Observer,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let visible = self.visible_for(view, observer.ship_id);
        self.delivery
            .deliver_frame(sink, view, observer, visible, new_events, warp_arrivals)
    }

    /// Drop per-player visible-set memory for sessions no longer owned by this
    /// Sector frame.
    pub fn retain_players(&mut self, keep: impl Fn(PlayerId) -> bool) {
        self.delivery.retain_players(keep);
    }

    fn visible_for<V: SectorView>(&self, view: &V, ship_id: ShipId) -> Vec<ShipId> {
        view.ship_absolute_pos(ship_id)
            .map(|position| self.index.neighbors_of(position))
            .unwrap_or_default()
    }
}

/// Transport-specific operations injected into [`deliver_sector_sessions`].
///
/// The callbacks keep `dawn-sector` independent from WebSocket and in-process
/// session types while the frame lifecycle remains shared by every runtime.
pub struct AoiSessionCallbacks<P, H, D, F> {
    /// Extract the player identity represented by a session.
    pub player_id: P,
    /// Extract the observed ship identity represented by a session.
    pub ship_id: H,
    /// Deliver the ordered frame and return whether the session remains live.
    pub deliver: D,
    /// Handle a committed jump before the session is removed from this Sector.
    pub on_redirect: F,
}

impl<P, H, D, F> std::fmt::Debug for AoiSessionCallbacks<P, H, D, F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AoiSessionCallbacks")
            .finish_non_exhaustive()
    }
}

/// Rebuild and deliver one Sector's AoI frame for all live sessions.
///
/// `jumped_ships` contains the ownership changes committed by the current
/// frame. Those sessions are removed before ordinary AoI delivery and handed
/// to the adapter through `on_redirect`. Passing an empty map gives the local
/// single-Sector runtime the same path without introducing transport logic.
pub fn deliver_sector_sessions<S, V, P, H, D, F>(
    frame: &mut AoiFrame,
    view: &V,
    sessions: &mut Vec<S>,
    new_events: &[DomainEvent],
    warp_arrivals: &[ShipId],
    jumped_ships: &HashMap<ShipId, SectorId>,
    callbacks: AoiSessionCallbacks<P, H, D, F>,
) where
    V: SectorView,
    P: FnMut(&S) -> PlayerId,
    H: FnMut(&S) -> ShipId,
    D: FnMut(&mut S, &mut AoiFrame, &V, &[DomainEvent], &[ShipId]) -> bool,
    F: FnMut(&mut S, SectorId),
{
    let AoiSessionCallbacks {
        mut player_id,
        mut ship_id,
        mut deliver,
        mut on_redirect,
    } = callbacks;
    frame.rebuild(view);
    sessions.retain_mut(|session| {
        if let Some(&destination) = jumped_ships.get(&ship_id(session)) {
            on_redirect(session, destination);
            let session_player_id = player_id(session);
            frame.retain_players(|candidate| candidate != session_player_id);
            return false;
        }

        deliver(session, frame, view, new_events, warp_arrivals)
    });

    let live: HashSet<PlayerId> = sessions.iter().map(&mut player_id).collect();
    frame.retain_players(|player_id| live.contains(&player_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aoi::AoiMessage;
    use crate::node::SimulationNode;
    use crate::view::SectorView;
    use dawn_core::{
        AbsolutePosition, NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, Velocity,
    };
    #[derive(Debug, Default, PartialEq, Eq)]
    struct FakeSink {
        enters: Vec<u64>,
        leaves: Vec<u64>,
    }

    impl AoiSink for FakeSink {
        fn send_aoi_message(&mut self, message: &AoiMessage) -> bool {
            match message {
                AoiMessage::AoiEnter(ship) => self.enters.push(ship.ship_id),
                AoiMessage::AoiLeave { ship_id } => self.leaves.push(*ship_id),
                AoiMessage::Fact(_)
                | AoiMessage::MotionCorrection { .. }
                | AoiMessage::PositionSnap { .. } => {}
            }
            true
        }
    }

    struct FakeView {
        ships: Vec<(ShipId, AbsolutePosition)>,
    }

    impl SectorView for FakeView {
        fn ship_absolute_positions(&self) -> Vec<(ShipId, AbsolutePosition)> {
            self.ships.clone()
        }

        fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<AbsolutePosition> {
            self.ships
                .iter()
                .find_map(|(id, position)| (*id == ship_id).then_some(*position))
        }

        fn ship_state(&self, _ship_id: ShipId) -> Option<dawn_protocol::ShipStateWire> {
            None
        }

        fn ship_is_warping(&self, _ship_id: ShipId) -> bool {
            false
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
    fn rebuilding_accepts_a_storage_free_sector_view() {
        let own = ShipId::new(NodeId(0), 1);
        let other = ShipId::new(NodeId(0), 2);
        let view = FakeView {
            ships: vec![
                (own, AbsolutePosition::ORIGIN),
                (other, AbsolutePosition::new(10.0, 0.0, 0.0)),
            ],
        };
        let mut frame = AoiFrame::new(100.0);

        frame.rebuild(&view);

        assert_eq!(frame.visible_for(&view, own), vec![own, other]);
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
