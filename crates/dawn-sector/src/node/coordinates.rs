//! Coordinate-composition accessors: `AnchorTable` (ADR-0029) callers on
//! behalf of `SimulationNode`. Ships store position as an anchor-relative f32
//! offset; every method here composes that with the ship's anchor into a
//! Sector-frame absolute position (or the inverse), so gameplay code never
//! reads a raw offset for cross-anchor geometry.

use dawn_core::{AbsolutePosition, Position, ShipId};
use dawn_ecs::{components::PositionComp, Entity};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Absolute position (Sector-frame) of a ship entity given its raw offset,
    /// composing its anchor (ADR-0029). f32 result (compressed-scale safe) —
    /// resolved via the f64 core (`entity_absolute_f64`) and cast down once,
    /// rather than composing in f32. Used by steering/AI code so positions
    /// across anchors are comparable; not for cross-anchor distance at
    /// true-AU scale (use `entity_absolute_f64`/`ship_distance` for that).
    pub(super) fn entity_absolute(&self, entity: Entity, offset: Position) -> Position {
        let a = self.entity_absolute_f64(entity, offset);
        Position::new(a[0] as f32, a[1] as f32, a[2] as f32)
    }

    /// Absolute (Sector-frame) position of a ship entity, composing its anchor
    /// with its current `PositionComp` offset (ADR-0029). The single accessor any
    /// gameplay code should use to compare a ship against Sector-frame data
    /// (gates, bodies, other ships) — never read the raw anchor-relative offset
    /// for cross-anchor geometry.
    pub(super) fn entity_abs_pos(&self, entity: Entity) -> Position {
        let off = self
            .world
            .get::<PositionComp>(entity)
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        self.entity_absolute(entity, off)
    }

    /// Absolute (Sector-frame, metres, f64) position of a ship entity, composing
    /// its anchor with its current `PositionComp` offset (ADR-0029). The f64
    /// counterpart of [`Self::entity_abs_pos`] — the AoI grid input that stays
    /// precise at true-AU distances (R2).
    pub(super) fn entity_abs_pos_f64(&self, entity: Entity) -> AbsolutePosition {
        let off = self
            .world
            .get::<PositionComp>(entity)
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        self.entity_absolute_f64(entity, off)
    }

    /// Absolute position (Sector-frame, metres, f64) of a ship entity given its
    /// raw offset, composing its anchor (ADR-0029). Used by warp arrival math
    /// that must stay precise at true-AU distances.
    pub(super) fn entity_absolute_f64(&self, entity: Entity, offset: Position) -> AbsolutePosition {
        let Some(anchor) = self.world.ship_anchor(entity) else {
            return offset.into();
        };
        let Some(abs) = self.anchor_table.absolute(anchor, offset) else {
            debug_assert_missing_anchor(anchor, "entity_absolute_f64");
            return offset.into();
        };
        abs
    }

    /// Convert a Sector-frame (absolute) destination given as an f64 point into
    /// the ship's current anchor frame (ADR-0029), doing the subtraction in f64
    /// before casting once — so it stays precise at true-AU distance from the
    /// ship's anchor. The inverse of `entity_absolute_f64`. Called from
    /// `approach.rs`, `commands.rs`, `orbit.rs` (arrival is a tight radius
    /// check, needs full precision) and from `warp.rs::dest_in_ship_frame`
    /// (which only has an already-f32 source, so is only as precise as that).
    pub(super) fn dest_in_ship_frame_abs(
        &self,
        entity: Entity,
        dest_abs: AbsolutePosition,
    ) -> Position {
        let Some(anchor) = self.world.ship_anchor(entity) else {
            return Position::new(dest_abs[0] as f32, dest_abs[1] as f32, dest_abs[2] as f32);
        };
        let Some(rel) = self.anchor_table.to_relative(anchor, dest_abs.into()) else {
            debug_assert_missing_anchor(anchor, "dest_in_ship_frame_abs");
            return Position::new(dest_abs[0] as f32, dest_abs[1] as f32, dest_abs[2] as f32);
        };
        rel
    }

    /// Distance from a Ship to a Sector-frame point (a gate/body position),
    /// composing the ship's anchor (ADR-0029). The accessor gameplay/tests use
    /// instead of comparing a raw offset to absolute data.
    pub fn ship_distance_to_point(&self, ship_id: ShipId, point: Position) -> Option<f32> {
        let entity = *self.ships.index.get(&ship_id)?;
        Some(self.entity_abs_pos(entity).distance(point))
    }

    /// True distance (metres) between two Ships, composing each ship's anchor
    /// and offset in f64 so the result is correct even if the two ships are
    /// anchored on different bodies (ADR-0029 step 3 / spike B-3). Resolves
    /// each ship to its `(AnchorId, offset)` and delegates the composition to
    /// `AnchorTable::distance`.
    pub fn ship_distance(&self, a: ShipId, b: ShipId) -> Option<f64> {
        let (anchor_a, off_a) = self.ship_anchor_and_offset(a)?;
        let (anchor_b, off_b) = self.ship_anchor_and_offset(b)?;
        self.anchor_table
            .distance((anchor_a, off_a), (anchor_b, off_b))
    }

    /// A ship's `AnchorId` and raw (anchor-relative) `PositionComp` offset —
    /// the pair `AnchorTable`'s composition methods take.
    fn ship_anchor_and_offset(&self, ship_id: ShipId) -> Option<(dawn_core::AnchorId, Position)> {
        let entity = *self.ships.index.get(&ship_id)?;
        let anchor = self.world.ship_anchor(entity)?;
        let offset = self.world.get::<PositionComp>(entity)?.0;
        Some((anchor, offset))
    }
}

/// Fire in debug builds when a ship's `AnchorComp` points at an `AnchorId` that
/// the `AnchorTable` doesn't know (ADR-0029 R3). The absolute-position accessors
/// fall back to treating the raw offset as absolute, which is only correct at the
/// Sector origin — at true-AU scale a missing anchor silently misplaces the ship
/// by the body's absolute position (~10^11 m). This can't happen for node-spawned
/// ships (every spawn anchors on a real table entry), so a miss means a data /
/// galaxy-table integrity bug; surface it loudly instead of returning a wrong
/// frame. Release builds keep the silent fallback as a safety net.
#[inline]
#[allow(clippy::assertions_on_constants)]
pub(super) fn debug_assert_missing_anchor(anchor: dawn_core::AnchorId, site: &str) {
    debug_assert!(
        false,
        "{site}: ship anchored on {anchor:?} which is absent from the AnchorTable \
         — absolute position fell back to the raw offset (wrong frame at true AU)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        AnchorId, DomainEvent, NodeId, SectorBounds, SectorId, ShipTypeId, Tick, Velocity,
    };

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn ship_distance_to_point_composes_the_ships_anchor() {
        let mut node = mem_node();
        let ship = node.spawn_ship(
            ShipTypeId(1),
            Position::new(1000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        // Rebase onto the star (AnchorId(0), abs [0,0,0]) so the expected
        // absolute position is exactly the offset -- ship_distance_to_point
        // is f32 (this file's own doc comment: not precise at true-AU
        // scale), so keep this near the origin rather than a far-away anchor
        // where a 10m difference would vanish in the f32 cast.
        node.apply_event_pub(DomainEvent::AnchorRebased(
            dawn_core::events::AnchorRebased {
                ship_id: ship,
                anchor: AnchorId(0),
                offset: Position::new(500.0, 0.0, 0.0),
                tick: Tick(1),
            },
        ));

        let point = Position::new(510.0, 0.0, 0.0);
        let dist = node.ship_distance_to_point(ship, point).unwrap();
        assert!(
            (dist - 10.0).abs() < 0.01,
            "expected ~10m, got {dist}m -- ship_distance_to_point must compose \
             the ship's anchor + offset before comparing to the Sector-frame point"
        );
    }

    #[test]
    fn ship_distance_to_point_returns_none_for_an_unknown_ship() {
        let node = mem_node();
        assert_eq!(
            node.ship_distance_to_point(dawn_core::ShipId::new(NodeId(0), 999), Position::ORIGIN),
            None
        );
    }
}
