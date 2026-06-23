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

use dawn_core::{DomainEvent, ShipId};

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
        assert!(
            cell_size > 0.0,
            "cell_size must be positive, got {cell_size}"
        );
        Self {
            cell_size,
            cells: BTreeMap::new(),
        }
    }

    /// Build a grid from `(ShipId, absolute position)` pairs. Positions are
    /// absolute (Sector-frame, metres) **f64** so the cell binning stays precise
    /// at true-AU distances — an f32 absolute near a 1e11 m anchor has a ~16 km
    /// ulp, which would fuzz the cell boundary (ADR-0029 R2). Each bucket is
    /// sorted by `ShipId` so neighborhood enumeration is deterministic.
    pub fn build(cell_size: f32, ships: impl IntoIterator<Item = (ShipId, [f64; 3])>) -> Self {
        let mut grid = Self::new(cell_size);
        for (id, pos) in ships {
            grid.cells
                .entry(grid_cell(cell_size, pos))
                .or_default()
                .push(id);
        }
        for bucket in grid.cells.values_mut() {
            bucket.sort_unstable();
        }
        grid
    }

    /// The cell an absolute position maps to (floor division on every axis).
    pub fn cell_of(&self, pos: [f64; 3]) -> Cell {
        grid_cell(self.cell_size, pos)
    }

    /// All `ShipId`s in the 3×3×3 neighborhood centered on `pos`'s cell, in
    /// `ShipId` order. Includes ships in the same cell as `pos`.
    pub fn neighbors_of(&self, pos: [f64; 3]) -> Vec<ShipId> {
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

/// Diff a previous visible set against the current one (both must be
/// `ShipId`-sorted, as returned by [`CellGrid::neighbors_of`]). Returns
/// `(entered, left)`: ships now visible that were not before, and ships no
/// longer visible. Both outputs stay `ShipId`-sorted. Used to emit `AoiEnter` /
/// `AoiLeave` to a client as it moves (ADR-0019).
pub fn aoi_delta(prev: &[ShipId], curr: &[ShipId]) -> (Vec<ShipId>, Vec<ShipId>) {
    let entered = curr
        .iter()
        .filter(|id| prev.binary_search(id).is_err())
        .copied()
        .collect();
    let left = prev
        .iter()
        .filter(|id| curr.binary_search(id).is_err())
        .copied()
        .collect();
    (entered, left)
}

/// The ships an event concerns — the actors a client must already know about for
/// the event to make sense. Primary ship plus any secondary participant
/// (attacker/target, killer). Used to decide whether to deliver an event to an
/// observer whose visible set is `visible` (ADR-0019).
pub fn event_concerns_ships(event: &DomainEvent) -> Vec<ShipId> {
    let mut ids = vec![event.ship_id()];
    let secondary = match event {
        DomainEvent::ShipDestroyed(e) => Some(e.killer_id),
        DomainEvent::TargetLocked(e) => Some(e.target_id),
        DomainEvent::LockLost(e) => Some(e.target_id),
        DomainEvent::WeaponFired(e) => Some(e.target_id),
        _ => None,
    };
    if let Some(s) = secondary {
        if s != ids[0] {
            ids.push(s);
        }
    }
    ids
}

/// Whether `event` should be delivered to an observer whose visible set is
/// `visible` (`ShipId`-sorted): true if any concerned ship is visible.
pub fn event_visible_to(event: &DomainEvent, visible: &[ShipId]) -> bool {
    event_concerns_ships(event)
        .iter()
        .any(|id| visible.binary_search(id).is_ok())
}

/// `AoiLeave` control message for a ship that left an observer's neighborhood
/// (ADR-0019). The matching `AoiEnter` needs node state, so it lives on the node
/// ([`crate::node::SimulationNode::aoi_enter_json`]).
pub fn aoi_leave_json(ship_id: ShipId) -> String {
    serde_json::json!({ "type": "AoiLeave", "ship_id": ship_id.raw() }).to_string()
}

/// Map an absolute (f64) position to its integer cell via floor division
/// (handles negatives: `-0.1` with size 100 → cell -1, not 0). The division is
/// done in f64 so a ship near a true-AU anchor lands in the right cell.
fn grid_cell(cell_size: f32, pos: [f64; 3]) -> Cell {
    let size = cell_size as f64;
    (
        (pos[0] / size).floor() as i32,
        (pos[1] / size).floor() as i32,
        (pos[2] / size).floor() as i32,
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
        assert_eq!(grid.cell_of([0.0, 0.0, 0.0]), (0, 0, 0));
        assert_eq!(grid.cell_of([150.0, 250.0, 50.0]), (1, 2, 0));
        // Negative coordinates floor toward minus infinity, not toward zero.
        assert_eq!(grid.cell_of([-0.1, -100.0, -100.1]), (-1, -1, -2));
    }

    #[test]
    fn a_cell_boundary_position_belongs_to_the_higher_cell() {
        let grid = CellGrid::new(100.0);
        // Exactly on the boundary (x = 100) floors into cell 1, deterministically.
        assert_eq!(grid.cell_of([100.0, 0.0, 0.0]), (1, 0, 0));
    }

    #[test]
    fn cell_binning_is_precise_at_true_au_where_f32_would_fuzz_the_boundary() {
        // Two ships 200 m apart straddling a cell boundary, sitting ~1 AU out.
        // In f64 they bin into adjacent cells (correct); cast through f32 first
        // (~16 km ulp at 1e11) they would collapse into the same coordinate and
        // mis-bin (ADR-0029 R2).
        const AU_M: f64 = 1.495978707e11;
        let grid = CellGrid::new(10_000.0);
        let boundary = (AU_M / 10_000.0).floor() * 10_000.0; // exact cell edge
        let lo = grid.cell_of([boundary - 100.0, 0.0, 0.0]);
        let hi = grid.cell_of([boundary + 100.0, 0.0, 0.0]);
        assert_eq!(
            hi.0,
            lo.0 + 1,
            "200 m across a cell edge at 1 AU must bin into adjacent cells"
        );
    }

    #[test]
    fn neighborhood_includes_same_cell_and_axis_adjacent_cells() {
        // center cell (0,0,0); one ship in it, one in an adjacent cell.
        let grid = CellGrid::build(
            100.0,
            [
                (ship(1), [10.0, 10.0, 10.0]),  // cell (0,0,0)
                (ship(2), [110.0, 10.0, 10.0]), // cell (1,0,0) — adjacent
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0]);
        assert_eq!(n, vec![ship(1), ship(2)]);
    }

    #[test]
    fn ships_two_cells_away_are_excluded() {
        let grid = CellGrid::build(
            100.0,
            [
                (ship(1), [50.0, 50.0, 50.0]),  // cell (0,0,0)
                (ship(2), [250.0, 50.0, 50.0]), // cell (2,0,0) — 2 cells away
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0]);
        assert_eq!(n, vec![ship(1)]);
    }

    #[test]
    fn enumeration_is_sorted_by_ship_id_regardless_of_insertion_order() {
        // Insert out of order and across several cells; result must be ShipId-sorted.
        let grid = CellGrid::build(
            100.0,
            [
                (ship(9), [10.0, 10.0, 10.0]),  // (0,0,0)
                (ship(3), [110.0, 10.0, 10.0]), // (1,0,0)
                (ship(5), [10.0, 110.0, 10.0]), // (0,1,0)
                (ship(1), [10.0, 10.0, 10.0]),  // (0,0,0)
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0]);
        assert_eq!(n, vec![ship(1), ship(3), ship(5), ship(9)]);
    }

    #[test]
    fn an_empty_grid_yields_no_neighbors() {
        let grid = CellGrid::build(100.0, std::iter::empty());
        assert!(grid.neighbors_of([0.0, 0.0, 0.0]).is_empty());
    }

    // ── AoI delta / event visibility ─────────────────────────────────────────

    #[test]
    fn aoi_delta_reports_entered_and_left_ships() {
        let prev = vec![ship(1), ship(2), ship(3)];
        let curr = vec![ship(2), ship(3), ship(5)]; // 1 left, 5 entered
        let (entered, left) = aoi_delta(&prev, &curr);
        assert_eq!(entered, vec![ship(5)]);
        assert_eq!(left, vec![ship(1)]);
    }

    #[test]
    fn aoi_delta_is_empty_when_the_visible_set_is_unchanged() {
        let set = vec![ship(1), ship(4)];
        let (entered, left) = aoi_delta(&set, &set);
        assert!(entered.is_empty() && left.is_empty());
    }

    #[test]
    fn an_event_is_visible_when_a_concerned_ship_is_in_the_set() {
        use dawn_core::events::WeaponFired;
        use dawn_core::Tick;
        // Attacker not visible, target visible → still delivered.
        let event = DomainEvent::WeaponFired(WeaponFired {
            attacker_id: ship(99),
            target_id: ship(2),
            damage: 10.0,
            tick: Tick(1),
        });
        let visible = vec![ship(1), ship(2), ship(3)];
        assert!(event_visible_to(&event, &visible));
    }

    #[test]
    fn an_event_is_hidden_when_no_concerned_ship_is_in_the_set() {
        use dawn_core::events::VelocityChanged;
        use dawn_core::{Tick, Velocity};
        let event = DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ship(99),
            velocity: Velocity::ZERO,
            tick: Tick(1),
        });
        let visible = vec![ship(1), ship(2)];
        assert!(!event_visible_to(&event, &visible));
    }
}
