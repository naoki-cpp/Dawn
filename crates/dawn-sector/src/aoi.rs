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

use std::collections::{BTreeMap, HashMap, HashSet};

use dawn_core::{AbsolutePosition, DomainEvent, PlayerId, ShipId};
use dawn_wire::{AbsPosWire, ServerMessage, VelWire};

use crate::view::SectorView;

/// Integer 3-D cell coordinate.
pub type Cell = (i32, i32, i32);

/// Static-cell spatial bucket for AoI queries.
#[derive(Debug)]
pub struct CellGrid {
    cell_size: f64,
    cells: BTreeMap<Cell, Vec<ShipId>>,
}

impl CellGrid {
    /// Create an empty grid with the given cell edge length.
    pub fn new(cell_size: f64) -> Self {
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
    pub fn build(
        cell_size: f64,
        ships: impl IntoIterator<Item = (ShipId, AbsolutePosition)>,
    ) -> Self {
        let mut grid = Self::new(cell_size);
        for (id, pos) in ships {
            grid.cells
                .entry(grid_cell(cell_size, pos.as_array()))
                .or_default()
                .push(id);
        }
        for bucket in grid.cells.values_mut() {
            bucket.sort_unstable();
        }
        grid
    }

    /// The cell an absolute position maps to (floor division on every axis).
    pub fn cell_of(&self, pos: AbsolutePosition) -> Cell {
        grid_cell(self.cell_size, pos.as_array())
    }

    /// All `ShipId`s in the 3×3×3 neighborhood centered on `pos`'s cell, in
    /// `ShipId` order. Includes ships in the same cell as `pos`.
    pub fn neighbors_of(&self, pos: AbsolutePosition) -> Vec<ShipId> {
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

/// The identity of one observer receiving a frame: which player, and which
/// ship is theirs (excluded from its own Enter/Leave/snap messages).
#[derive(Debug)]
pub struct Observer {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

/// Per-session delivery destination for one Area-of-Interest frame. Two real
/// adapters justify this seam: the in-process serve loop's `PlayerSession`
/// and the standalone sector-node binary's `PlayerSession` — both wrap the
/// same `dawn-actor` connection type, but `dawn-sector` cannot name it
/// directly without depending on `dawn-actor` (see AI_DEVELOPMENT_GUIDE.md
/// crate boundaries). Callers wrap their session in a local newtype that
/// implements this trait (orphan-rule friendly: the wrapper type is local to
/// the calling crate even though the trait lives here).
pub trait AoiSink {
    fn send_events(&mut self, events: &[DomainEvent]) -> bool;
    fn send_message(&mut self, msg: &ServerMessage) -> bool;
}

/// Area-of-Interest delivery policy (ADR-0019): tracks each player's visible
/// set across ticks and emits `AoiEnter` / `AoiLeave` plus the new events and
/// warp-arrival snaps a client is owed for one frame. Owns the per-player
/// visible-set memory so callers don't have to.
///
/// Out of scope (caller's responsibility): cross-sector Redirect handling,
/// session retention/removal, and building the `CellGrid` for the tick —
/// this struct only delivers a frame given an already-computed current
/// visible set.
#[derive(Debug, Default)]
pub struct AoiDelivery {
    visible_by_player: HashMap<PlayerId, Vec<ShipId>>,
}

impl AoiDelivery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed (or reseed) a player's visible set without emitting any
    /// Enter/Leave messages — used when a player joins or transfers sectors
    /// in-process and the first frame should not look like every ship just
    /// entered.
    pub fn seed_player(&mut self, player_id: PlayerId, visible: Vec<ShipId>) {
        self.visible_by_player.insert(player_id, visible);
    }

    /// Drop visible-set memory for players no longer present, given the
    /// current player set.
    pub fn retain_players(&mut self, keep: impl Fn(PlayerId) -> bool) {
        self.visible_by_player.retain(|pid, _| keep(*pid));
    }

    /// Deliver one Area-of-Interest frame to `sink`: `AoiEnter` for ships
    /// that just became visible, `AoiLeave` for ships that left, the new
    /// domain events that concern a currently-visible ship, then any
    /// warp-arrival `PositionSnap`s relevant to this observer. Returns
    /// `false` as soon as a send fails (caller should drop the session).
    pub fn deliver_frame<V: SectorView>(
        &mut self,
        sink: &mut dyn AoiSink,
        view: &V,
        observer: Observer,
        curr: Vec<ShipId>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let Observer {
            player_id,
            ship_id: own_ship_id,
        } = observer;
        // Ships that have a ShipDestroyed event this tick must NOT receive an
        // AoiLeave -- the client's _handle_ship_destroyed already removes them.
        let destroyed_this_tick: HashSet<ShipId> = new_events
            .iter()
            .filter_map(|e| {
                if let DomainEvent::ShipDestroyed(d) = e {
                    Some(d.ship_id)
                } else {
                    None
                }
            })
            .collect();

        let old_prev = self
            .visible_by_player
            .get(&player_id)
            .cloned()
            .unwrap_or_default();
        let (entered, left) = aoi_delta(&old_prev, &curr);
        self.seed_player(player_id, curr.clone());

        for &id in entered.iter().filter(|&&id| id != own_ship_id) {
            if let Some(ship) = view.ship_state(id) {
                if !sink.send_message(&ServerMessage::AoiEnter(ship)) {
                    return false;
                }
            }
        }
        for &id in left
            .iter()
            .filter(|&&id| id != own_ship_id && !destroyed_this_tick.contains(&id))
        {
            if !sink.send_message(&ServerMessage::AoiLeave { ship_id: id.raw() }) {
                return false;
            }
        }

        let mut event_visible = curr.clone();
        if event_visible.binary_search(&own_ship_id).is_err() {
            event_visible.push(own_ship_id);
            event_visible.sort_unstable_by_key(|id| id.raw());
        }

        let visible_events: Vec<_> = new_events
            .iter()
            .filter(|e| {
                if let DomainEvent::ShipDestroyed(d) = e {
                    return old_prev.binary_search(&d.ship_id).is_ok()
                        || old_prev.binary_search(&d.killer_id).is_ok()
                        || event_visible_to(e, &event_visible);
                }
                event_visible_to(e, &event_visible)
            })
            .cloned()
            .collect();
        if !sink.send_events(&visible_events) {
            return false;
        }

        // Client-side prediction authority (ADR-0043): the owning client gets
        // an exact normal-flight position/velocity correction alongside the
        // velocity event. Committed warp is deliberately excluded because its
        // visual path is capped and its arrival is corrected by PositionSnap.
        let owner_motion = new_events.iter().rev().find_map(|event| match event {
            DomainEvent::VelocityChanged(change) if change.ship_id == own_ship_id => {
                Some((change.velocity, change.tick))
            }
            _ => None,
        });
        if !view.ship_is_warping(own_ship_id)
            && !warp_arrivals.contains(&own_ship_id)
            && owner_motion.is_some()
        {
            if let (Some(position), Some((velocity, tick))) =
                (view.ship_absolute_pos(own_ship_id), owner_motion)
            {
                let msg = ServerMessage::MotionCorrection {
                    ship_id: own_ship_id.raw(),
                    position: AbsPosWire {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    },
                    velocity: VelWire::from(velocity),
                    tick: tick.0,
                };
                if !sink.send_message(&msg) {
                    return false;
                }
            }
        }

        // Warp-arrival authority (ADR-0029): a warp ends with the client's
        // visual ship lagging behind. Push the server's absolute arrival as a
        // `PositionSnap` to the owner and any observer that can see the ship.
        for &sid in warp_arrivals {
            let relevant = sid == own_ship_id || curr.binary_search(&sid).is_ok();
            if relevant {
                if let Some(abs) = view.ship_absolute_pos(sid) {
                    let msg = ServerMessage::PositionSnap {
                        ship_id: sid.raw(),
                        position: AbsPosWire {
                            x: abs[0],
                            y: abs[1],
                            z: abs[2],
                        },
                    };
                    if !sink.send_message(&msg) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Map an absolute (f64) position to its integer cell via floor division
/// (handles negatives: `-0.1` with size 100 → cell -1, not 0). The division is
/// done in f64 so a ship near a true-AU anchor lands in the right cell.
fn grid_cell(cell_size: f64, pos: [f64; 3]) -> Cell {
    let size = cell_size;
    (
        (pos[0] / size).floor() as i32,
        (pos[1] / size).floor() as i32,
        (pos[2] / size).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::SimulationNode;
    use dawn_core::NodeId;
    use dawn_wire::ShipStateWire;

    fn ship(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    #[test]
    fn cell_assignment_uses_floor_division_including_negatives() {
        let grid = CellGrid::new(100.0);
        assert_eq!(grid.cell_of([0.0, 0.0, 0.0].into()), (0, 0, 0));
        assert_eq!(grid.cell_of([150.0, 250.0, 50.0].into()), (1, 2, 0));
        // Negative coordinates floor toward minus infinity, not toward zero.
        assert_eq!(grid.cell_of([-0.1, -100.0, -100.1].into()), (-1, -1, -2));
    }

    #[test]
    fn a_cell_boundary_position_belongs_to_the_higher_cell() {
        let grid = CellGrid::new(100.0);
        // Exactly on the boundary (x = 100) floors into cell 1, deterministically.
        assert_eq!(grid.cell_of([100.0, 0.0, 0.0].into()), (1, 0, 0));
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
        let lo = grid.cell_of([boundary - 100.0, 0.0, 0.0].into());
        let hi = grid.cell_of([boundary + 100.0, 0.0, 0.0].into());
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
                (ship(1), [10.0, 10.0, 10.0].into()),  // cell (0,0,0)
                (ship(2), [110.0, 10.0, 10.0].into()), // cell (1,0,0) — adjacent
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0].into());
        assert_eq!(n, vec![ship(1), ship(2)]);
    }

    #[test]
    fn ships_two_cells_away_are_excluded() {
        let grid = CellGrid::build(
            100.0,
            [
                (ship(1), [50.0, 50.0, 50.0].into()),  // cell (0,0,0)
                (ship(2), [250.0, 50.0, 50.0].into()), // cell (2,0,0) — 2 cells away
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0].into());
        assert_eq!(n, vec![ship(1)]);
    }

    #[test]
    fn enumeration_is_sorted_by_ship_id_regardless_of_insertion_order() {
        // Insert out of order and across several cells; result must be ShipId-sorted.
        let grid = CellGrid::build(
            100.0,
            [
                (ship(9), [10.0, 10.0, 10.0].into()),  // (0,0,0)
                (ship(3), [110.0, 10.0, 10.0].into()), // (1,0,0)
                (ship(5), [10.0, 110.0, 10.0].into()), // (0,1,0)
                (ship(1), [10.0, 10.0, 10.0].into()),  // (0,0,0)
            ],
        );
        let n = grid.neighbors_of([50.0, 50.0, 50.0].into());
        assert_eq!(n, vec![ship(1), ship(3), ship(5), ship(9)]);
    }

    #[test]
    fn an_empty_grid_yields_no_neighbors() {
        let grid = CellGrid::build(100.0, std::iter::empty());
        assert!(grid.neighbors_of([0.0, 0.0, 0.0].into()).is_empty());
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

    // ── AoiDelivery ───────────────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeSink {
        events: Vec<DomainEvent>,
        aoi_enters: Vec<ShipStateWire>,
        aoi_leaves: Vec<u64>,
        position_snaps: Vec<(u64, AbsPosWire)>,
        motion_corrections: Vec<(u64, AbsPosWire, VelWire, u64)>,
    }

    impl AoiSink for FakeSink {
        fn send_events(&mut self, events: &[DomainEvent]) -> bool {
            self.events.extend_from_slice(events);
            true
        }
        fn send_message(&mut self, msg: &ServerMessage) -> bool {
            match msg {
                ServerMessage::AoiEnter(ship) => self.aoi_enters.push(ship.clone()),
                ServerMessage::AoiLeave { ship_id } => self.aoi_leaves.push(*ship_id),
                ServerMessage::PositionSnap { ship_id, position } => {
                    self.position_snaps.push((*ship_id, *position))
                }
                ServerMessage::MotionCorrection {
                    ship_id,
                    position,
                    velocity,
                    tick,
                } => self
                    .motion_corrections
                    .push((*ship_id, *position, *velocity, *tick)),
                _ => {}
            }
            true
        }
    }

    fn mem_node() -> SimulationNode {
        use dawn_core::{SectorBounds, SectorId};
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn player(n: u64) -> PlayerId {
        PlayerId(n)
    }

    #[test]
    fn deliver_frame_emits_aoi_enter_for_a_newly_visible_ship() {
        use dawn_core::{Position, Velocity};
        let mut node = mem_node();
        let other = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );

        let mut delivery = AoiDelivery::new();
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: ship(1),
            },
            vec![other],
            &[],
            &[],
        );

        assert_eq!(
            sink.aoi_enters.len(),
            1,
            "exactly one AoiEnter for the new ship"
        );
        assert_eq!(sink.aoi_enters[0].ship_id, other.raw());
    }

    #[test]
    fn deliver_frame_emits_aoi_leave_for_a_ship_that_left_unless_it_was_destroyed() {
        let node = mem_node();
        let mut delivery = AoiDelivery::new();
        delivery.seed_player(player(1), vec![ship(2), ship(3)]);

        // ship(2) left and was NOT destroyed -> AoiLeave.
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: ship(1),
            },
            vec![ship(3)],
            &[],
            &[],
        );
        assert_eq!(sink.aoi_leaves, vec![ship(2).raw()]);

        // Reseed, then ship(2) leaves because it was destroyed this tick -> no AoiLeave.
        delivery.seed_player(player(1), vec![ship(2), ship(3)]);
        let destroyed = DomainEvent::ShipDestroyed(dawn_core::events::ShipDestroyed {
            ship_id: ship(2),
            killer_id: ship(3),
            tick: dawn_core::Tick(1),
        });
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: ship(1),
            },
            vec![ship(3)],
            &[destroyed],
            &[],
        );
        assert!(
            sink.aoi_leaves.is_empty(),
            "a destroyed ship's own removal handles this client-side, no AoiLeave"
        );
    }

    #[test]
    fn deliver_frame_sends_own_ship_events_even_when_visible_set_excludes_self() {
        use dawn_core::events::ModuleActivated;
        use dawn_core::{ModuleId, SlotKind, Tick};

        let node = mem_node();
        let own_ship = ship(1);
        let event = DomainEvent::ModuleActivated(ModuleActivated {
            ship_id: own_ship,
            module_id: ModuleId(7),
            slot: SlotKind::Mid,
            target_ship_id: None,
            tick: Tick(1),
        });

        let mut delivery = AoiDelivery::new();
        delivery.seed_player(player(1), vec![]);
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: own_ship,
            },
            vec![],
            std::slice::from_ref(&event),
            &[],
        );

        assert_eq!(
            sink.events,
            vec![event],
            "the owning client must receive its own module event so HUD state updates"
        );
    }

    #[test]
    fn deliver_frame_sends_motion_correction_for_the_own_ship() {
        use dawn_core::{Position, Tick, Velocity};
        let mut node = mem_node();
        let own_ship = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 20.0, 30.0),
            Velocity::ZERO,
        );
        let event = DomainEvent::VelocityChanged(dawn_core::events::VelocityChanged {
            ship_id: own_ship,
            velocity: Velocity::new(4.0, 5.0, 6.0),
            tick: Tick(9),
        });

        let mut delivery = AoiDelivery::new();
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: own_ship,
            },
            vec![],
            std::slice::from_ref(&event),
            &[],
        );

        assert_eq!(sink.motion_corrections.len(), 1);
        assert_eq!(sink.motion_corrections[0].0, own_ship.raw());
        assert_eq!(sink.motion_corrections[0].1.x, 10.0);
        assert_eq!(
            sink.motion_corrections[0].2,
            VelWire::from(Velocity::new(4.0, 5.0, 6.0))
        );
        assert_eq!(sink.motion_corrections[0].3, 9);
    }

    #[test]
    fn deliver_frame_pushes_a_position_snap_for_a_visible_warp_arrival() {
        use dawn_core::{Position, Velocity};
        let mut node = mem_node();
        let arrived = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(5.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let mut delivery = AoiDelivery::new();
        delivery.seed_player(player(1), vec![arrived]);
        let mut sink = FakeSink::default();
        delivery.deliver_frame(
            &mut sink,
            &node,
            Observer {
                player_id: player(1),
                ship_id: ship(1),
            },
            vec![arrived],
            &[],
            &[arrived],
        );

        assert_eq!(
            sink.position_snaps.len(),
            1,
            "a PositionSnap is sent for the visible warp arrival"
        );
        assert_eq!(sink.position_snaps[0].0, arrived.raw());
    }
}
