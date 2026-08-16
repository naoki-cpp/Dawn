//! Coordinate-composition accessors: `AnchorTable` (ADR-0029) callers on
//! behalf of `SimulationNode`. Ships store position as an anchor-relative f64
//! offset; every method here composes that with the ship's anchor into a
//! Sector-frame absolute position (or the inverse), so gameplay code never
//! reads a raw offset for cross-anchor geometry.

use dawn_core::{AbsolutePosition, Position, ShipId};
use dawn_ecs::{components::PositionComp, Entity};

use super::SimulationNode;

impl SimulationNode {
    /// Absolute position (Sector-frame) of a ship entity given its raw offset,
    /// composing its anchor (ADR-0029) without narrowing the result.
    pub(super) fn entity_absolute(&self, entity: Entity, offset: Position) -> Position {
        let a = self.entity_absolute_f64(entity, offset);
        Position::new(a[0], a[1], a[2])
    }

    /// Absolute (Sector-frame, metres, f64) position of a ship entity, composing
    /// its anchor with its current `PositionComp` offset (ADR-0029). The f64
    /// input that stays precise at true-AU distances (R2), used as the AoI
    /// grid input.
    pub(super) fn entity_abs_pos_f64(&self, entity: Entity) -> AbsolutePosition {
        let off = self
            .simulation
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
        let Some(anchor) = self.simulation.world.ship_anchor(entity) else {
            return offset.into();
        };
        let Some(abs) = self.topology.anchor_table.absolute(anchor, offset) else {
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
    /// (which receives the same f64 source).
    pub(super) fn dest_in_ship_frame_abs(
        &self,
        entity: Entity,
        dest_abs: AbsolutePosition,
    ) -> Position {
        let Some(anchor) = self.simulation.world.ship_anchor(entity) else {
            return Position::new(dest_abs[0], dest_abs[1], dest_abs[2]);
        };
        let Some(rel) = self.topology.anchor_table.to_relative(anchor, dest_abs) else {
            debug_assert_missing_anchor(anchor, "dest_in_ship_frame_abs");
            return Position::new(dest_abs[0], dest_abs[1], dest_abs[2]);
        };
        rel
    }

    /// True distance (metres) between two Ships, composing each ship's anchor
    /// and offset in f64 so the result is correct even if the two ships are
    /// anchored on different bodies (ADR-0029 step 3 / spike B-3). Resolves
    /// each ship to its `(AnchorId, offset)` and delegates the composition to
    /// `AnchorTable::distance`.
    pub(crate) fn ship_distance(&self, a: ShipId, b: ShipId) -> Option<f64> {
        let (anchor_a, off_a) = self.ship_anchor_and_offset(a)?;
        let (anchor_b, off_b) = self.ship_anchor_and_offset(b)?;
        self.topology
            .anchor_table
            .distance((anchor_a, off_a), (anchor_b, off_b))
    }

    /// A ship's `AnchorId` and raw (anchor-relative) `PositionComp` offset —
    /// the pair `AnchorTable`'s composition methods take.
    fn ship_anchor_and_offset(&self, ship_id: ShipId) -> Option<(dawn_core::AnchorId, Position)> {
        let entity = *self.simulation.ships.index.get(&ship_id)?;
        let anchor = self.simulation.world.ship_anchor(entity)?;
        let offset = self.simulation.world.get::<PositionComp>(entity)?.0;
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
