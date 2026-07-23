//! Pre-Raft navigation validation for `SimulationNode` (INV-006).
//!
//! `can_propose_jump` / `can_propose_warp` reject a `JumpCommand` / `WarpCommand`
//! up front, before proposing to the Raft Log. The per-tick systems and command
//! handlers that act on an accepted command live in sibling modules (ADR-0030
//! review R-1, 2026-06-23 — split out of this file, which had grown to 1092
//! lines mixing three responsibilities):
//!
//! - `node::approach` — `apply_approach_command(_owned)` / `process_approach` (ADR-0015)
//! - `node::warp`      — `apply_warp_command(_owned)` / `process_warp` (ADR-0022/0023/0025/0029)

use dawn_core::{JumpGateId, Position, ShipId, WarpTarget};
use dawn_ecs::components::PositionComp;
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Whether a `JumpCommand` for `ship_id` via `gate_id` would currently be
    /// accepted: the Ship exists, is not already in transit, the gate
    /// originates in this Sector, and the Ship is within its `activation_radius`.
    /// Used to reject commands up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        if self.world.is_tackled(entity) {
            return false;
        }
        let Some(gate) = self.sector_map.gates.get(&gate_id) else {
            return false;
        };
        // Compare in absolute (Sector-frame) f64 coords: the gate's f64 `abs_m` is
        // Sector-frame, the ship offset is anchor-relative. f64 keeps the check
        // precise at true-AU distances (ADR-0029 review R1 / #4).
        let offset = self
            .world
            .get::<PositionComp>(entity)
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        gate.is_in_range_abs(self.entity_absolute_f64(entity, offset).into())
    }

    /// Whether a `WarpCommand` for `ship_id` toward `target` would currently be
    /// accepted (INV-006 Validation, before attaching `WarpComp`):
    /// the Ship exists, is not in transit, is not already warping, not tackled,
    /// the target belongs to this Sector, and is at least `MIN_WARP_DISTANCE` away.
    pub fn can_propose_warp(&self, ship_id: ShipId, target: WarpTarget) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        if self
            .world
            .get::<dawn_ecs::components::WarpComp>(entity)
            .is_some()
        {
            return false;
        }
        if self.world.is_tackled(entity) {
            return false;
        }
        // Absolute (Sector-frame) f64 ship position vs the f64 gate/body source
        // (ADR-0029 R1 / #4 — never compare a raw anchor offset to absolute data,
        // and keep the distance precise at true-AU scale).
        let offset = self
            .world
            .get::<PositionComp>(entity)
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        let ship_abs = self.entity_absolute_f64(entity, offset).into();
        let min = super::MIN_WARP_DISTANCE as f64;
        match target {
            WarpTarget::Gate(gate_id) => {
                let Some(gate) = self.sector_map.gates.get(&gate_id) else {
                    return false;
                };
                gate.distance_abs(ship_abs) >= min
            }
            WarpTarget::Body(body_id) => {
                let Some(body) = self.sector_map.bodies.get(&body_id) else {
                    return false;
                };
                let d = [
                    ship_abs[0] - body.abs_m[0],
                    ship_abs[1] - body.abs_m[1],
                    ship_abs[2] - body.abs_m[2],
                ];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() >= min
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, SectorBounds, SectorId};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    /// Reproduces the bug a player reported as "gate jump sometimes fails even
    /// in range": every demo/production gate has `activation_radius` (2000u)
    /// smaller than `MIN_WARP_DISTANCE` (3000u), so a ship sitting in that
    /// 1000u band is too far for `can_propose_jump` but too close for
    /// `apply_warp_command`'s own distance check. Before the fix, a
    /// `JumpCommand` issued from this band was silently dropped every tick
    /// with no fallback and no feedback -- this test pins the gap so the
    /// dispatch sites (cluster.rs / dawn-sector-node/main.rs) know an Approach
    /// fallback is available and must use it.
    #[test]
    fn ship_between_activation_radius_and_min_warp_distance_can_neither_jump_nor_warp_but_can_approach(
    ) {
        let mut node = mem_node();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        assert!(
            gate.activation_radius < super::super::MIN_WARP_DISTANCE,
            "this test only makes sense while the gap exists"
        );
        let gap_distance =
            (gate.activation_radius as f64 + super::super::MIN_WARP_DISTANCE as f64) / 2.0;
        let gap_abs = [gate.abs_m[0] - gap_distance, gate.abs_m[1], gate.abs_m[2]];

        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, gap_abs);

        assert!(
            !node.can_propose_jump(ship, JumpGateId(0)),
            "gap distance must be outside activation_radius"
        );
        assert!(
            !node.apply_warp_command(ship, WarpTarget::Gate(JumpGateId(0)), true),
            "gap distance must be closer than MIN_WARP_DISTANCE"
        );
        assert!(
            node.apply_approach_command(ship, dawn_core::ApproachTarget::Gate(JumpGateId(0))),
            "Approach must remain available as the fallback that closes the gap"
        );
        let _ = player;
    }
}
