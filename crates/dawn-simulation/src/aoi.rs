//! Static cell grid for Area-of-Interest (ADR-0019).
//!
//! Space is partitioned by a fixed grid (floor-division into integer cells).
//! Each cell holds the `ShipId`s currently inside it. A player's interest region
//! is its own cell plus every axis-adjacent cell — the 3×3×3 = 27-cell
//! neighborhood in 3-D (ADR-0019 wrote "3×3 / 9 cells" in 2-D shorthand; the
//! world is 3-D, so the neighborhood is 27 cells). Putting the discontinuity at
//! the 3×3×3 outer shell keeps it ~1.5 cells from the observer.
//!
//! # Derived / non-persistent (INV-002 / INV-MOVE)
//!
//! The grid is rebuilt from ship positions (themselves derived, transient state)
//! and is never serialised or stored in a snapshot — exactly like position. After
//! recovery it is repopulated by the live sim. It carries no authority: it only
//! decides what is *delivered* to a client, never what happens in the world
//! (ADR-0019). Enumeration is `ShipId`-sorted so the result is deterministic
//! regardless of insertion order.

use std::collections::BTreeMap;

use dawn_core::{Position, ShipId};

/// Integer 3-D cell coordinate.
pub type Cell = (i32, i32, i32);

/// Static-cell spatial bucket for AoI queries.
pub struct CellGrid {
    cell_size: f32,
    cells: BTreeMap<Cell, Vec<ShipId>>,
}

impl CellGrid {
    /// Create an empty grid with the given cell edge length.
    pub fn new(cell_size: f32) -> Self {
        assert!(cell_size > 0.0, "cell_size must be positive, got {cell_size}");
        Self { cell_size, cells: BTreeMap::new() }
    }

    /// Build a grid from `(ShipId, Position)` pairs. Each bucket is sorted by
    /// `ShipId` so neighborhood enumeration is deterministic.
    pub fn build(
        cell_size: f32,
        ships: impl IntoIterator<Item = (ShipId, Position)>,
    ) -> Self {
        let mut grid = Self::new(cell_size);
        for (id, pos) in ships {
            grid.cells.entry(grid_cell(cell_size, pos)).or_default().push(id);
        }
        for bucket in grid.cells.values_mut() {
            bucket.sort_unstable();
        }
        grid
    }

    /// The cell a position maps to (floor division on every axis).
    pub fn cell_of(&self, pos: Position) -> Cell {
        grid_cell(self.cell_size, pos)
    }

    /// All `ShipId`s in the 3×3×3 neighborhood centered on `pos`'s cell, in
    /// `ShipId` order. Includes ships in the same cell as `pos`.
    pub fn neighbors_of(&self, pos: Position) -> Vec<ShipId> {
        let (cx, cy, cz) = self.cell_of(pos);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(bucket) = self.cells.get(&(cx + dx, cy + dy, cz + dz)) {
                        out.extend_from_slice(bucket);
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }
}

/// Map a position to its integer cell via floor division (handles negatives:
/// `-0.1` with size 100 → cell -1, not 0).
fn grid_cell(cell_size: f32, pos: Position) -> Cell {
    (
        (pos.x / cell_size).floor() as i32,
        (pos.y / cell_size).floor() as i32,
        (pos.z / cell_size).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;

    fn ship(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    #[test]
    fn cell_assignment_uses_floor_division_including_negatives() {
        let grid = CellGrid::new(100.0);
        assert_eq!(grid.cell_of(Position::new(0.0, 0.0, 0.0)), (0, 0, 0));
        assert_eq!(grid.cell_of(Position::new(150.0, 250.0, 50.0)), (1, 2, 0));
        // Negative coordinates floor toward minus infinity, not toward zero.
        assert_eq!(grid.cell_of(Position::new(-0.1, -100.0, -100.1)), (-1, -1, -2));
    }

    #[test]
    fn a_cell_boundary_position_belongs_to_the_higher_cell() {
        let grid = CellGrid::new(100.0);
        // Exactly on the boundary (x = 100) floors into cell 1, deterministically.
        assert_eq!(grid.cell_of(Position::new(100.0, 0.0, 0.0)), (1, 0, 0));
    }

    #[test]
    fn neighborhood_includes_same_cell_and_axis_adjacent_cells() {
        // center cell (0,0,0); one ship in it, one in an adjacent cell.
        let grid = CellGrid::build(100.0, [
            (ship(1), Position::new(10.0, 10.0, 10.0)),    // cell (0,0,0)
            (ship(2), Position::new(110.0, 10.0, 10.0)),   // cell (1,0,0) — adjacent
        ]);
        let n = grid.neighbors_of(Position::new(50.0, 50.0, 50.0));
        assert_eq!(n, vec![ship(1), ship(2)]);
    }

    #[test]
    fn ships_two_cells_away_are_excluded() {
        let grid = CellGrid::build(100.0, [
            (ship(1), Position::new(50.0, 50.0, 50.0)),    // cell (0,0,0)
            (ship(2), Position::new(250.0, 50.0, 50.0)),   // cell (2,0,0) — 2 cells away
        ]);
        let n = grid.neighbors_of(Position::new(50.0, 50.0, 50.0));
        assert_eq!(n, vec![ship(1)]);
    }

    #[test]
    fn enumeration_is_sorted_by_ship_id_regardless_of_insertion_order() {
        // Insert out of order and across several cells; result must be ShipId-sorted.
        let grid = CellGrid::build(100.0, [
            (ship(9), Position::new(10.0, 10.0, 10.0)),    // (0,0,0)
            (ship(3), Position::new(110.0, 10.0, 10.0)),   // (1,0,0)
            (ship(5), Position::new(10.0, 110.0, 10.0)),   // (0,1,0)
            (ship(1), Position::new(10.0, 10.0, 10.0)),    // (0,0,0)
        ]);
        let n = grid.neighbors_of(Position::new(50.0, 50.0, 50.0));
        assert_eq!(n, vec![ship(1), ship(3), ship(5), ship(9)]);
    }

    #[test]
    fn an_empty_grid_yields_no_neighbors() {
        let grid = CellGrid::build(100.0, std::iter::empty());
        assert!(grid.neighbors_of(Position::ORIGIN).is_empty());
    }
}
