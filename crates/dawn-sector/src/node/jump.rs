//! Jump-gate command orchestration for `SimulationNode` (ADR-0009/0022).
//!
//! Owns two entry points into the jump pipeline:
//!
//! - `apply_jump_with_fallback` — player-initiated `JumpCommand`: tries the
//!   Transit / auto-warp / approach fallback chain in order and returns which
//!   path fired as a `JumpOutcome`.
//! - `resolve_auto_jump` — auto-jump on warp/approach arrival: checks whether
//!   the ship is now in range and returns the destination `SectorId` so the
//!   caller can propose Transit to Raft, or `None` to silently drop.
//!
//! Both callers outside `dawn-sector` (the production tick loops) only need to
//! know the result; the game-rule conditions stay here.

use dawn_core::{JumpGateId, SectorId, ShipId, WarpTarget};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

/// What a `JumpCommand` resolved to (ADR-0009 §5 / ADR-0022 §2 fallback chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpOutcome {
    /// The ship is within the gate's activation radius. The caller must
    /// propose a `TransitOp::Request { ship_id, to, gate_id: Some(gate_id) }`
    /// to Raft (the consensus handle lives outside `dawn-sector`).
    NeedsTransitProposal { to: SectorId },
    /// Out of range: an auto-warp toward the gate was started instead
    /// (`auto_jump = true`, so arrival queues a follow-up jump attempt).
    WarpFallbackStarted,
    /// Too close to warp: an approach toward the gate was started instead
    /// (arrival queues the same follow-up jump attempt as the warp fallback).
    ApproachFallbackStarted,
    /// The ship or gate doesn't exist, or every fallback was rejected (e.g.
    /// the ship is tackled or already mid-maneuver under Warp priority).
    Rejected,
}

impl<S: EventStore> SimulationNode<S> {
    /// If `ship_id` has arrived within `gate_id`'s activation radius, return
    /// the destination `SectorId` so the caller can propose a Transit to Raft.
    /// Returns `None` if not yet in range — the attempt is silently dropped;
    /// auto-jump never retries, it fires once on warp/approach arrival.
    ///
    /// Ownership checks are the caller's responsibility. Tackle blocks
    /// auto-jump: CONTEXT.md defines Tackle as preventing escape actions
    /// including jump.
    pub fn resolve_auto_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> Option<SectorId> {
        if !self.can_propose_jump(ship_id, gate_id) {
            return None;
        }
        Some(
            self.jump_gate(gate_id)
                .expect("can_propose_jump confirmed the gate exists")
                .to_sector,
        )
    }

    /// Resolve a `JumpCommand` against `gate_id`, trying the Transit /
    /// auto-warp / approach fallback chain in order (ADR-0009 §5, ADR-0022
    /// §2). Ownership checks are the caller's responsibility -- this only
    /// knows about the ship and the gate, not which player session issued
    /// the command.
    pub fn apply_jump_with_fallback(
        &mut self,
        ship_id: ShipId,
        gate_id: JumpGateId,
    ) -> JumpOutcome {
        if self.can_propose_jump(ship_id, gate_id) {
            let to = self
                .jump_gate(gate_id)
                .expect("can_propose_jump confirmed the gate exists")
                .to_sector;
            return JumpOutcome::NeedsTransitProposal { to };
        }
        if self.apply_warp_command(ship_id, WarpTarget::Gate(gate_id), true) {
            return JumpOutcome::WarpFallbackStarted;
        }
        if self.apply_approach_command_with_auto_jump(
            ship_id,
            dawn_core::ApproachTarget::Gate(gate_id),
            Some(gate_id),
        ) {
            return JumpOutcome::ApproachFallbackStarted;
        }
        JumpOutcome::Rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId};

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

    #[test]
    fn in_range_ship_resolves_to_a_transit_proposal() {
        let mut node = mem_node();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        let near_gate_abs = [
            gate.abs_m[0] - (gate.activation_radius as f64 * 0.5),
            gate.abs_m[1],
            gate.abs_m[2],
        ];
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, near_gate_abs);
        assert!(
            node.can_propose_jump(ship, JumpGateId(0)),
            "fixture must start within the gate's activation radius"
        );

        let outcome = node.apply_jump_with_fallback(ship, JumpGateId(0));
        assert_eq!(
            outcome,
            JumpOutcome::NeedsTransitProposal { to: gate.to_sector }
        );
        assert!(
            node.warp_phase(ship).is_none(),
            "in-range jump must not also start a warp"
        );
        assert_eq!(
            node.approach_target(ship),
            None,
            "in-range jump must not also start an approach"
        );
    }

    #[test]
    fn out_of_range_ship_falls_back_to_warp() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(
            !node.can_propose_jump(ship, JumpGateId(0)),
            "fixture starts far from the gate"
        );

        let outcome = node.apply_jump_with_fallback(ship, JumpGateId(0));
        assert_eq!(outcome, JumpOutcome::WarpFallbackStarted);
        assert!(
            node.warp_phase(ship).is_some(),
            "warp fallback must attach a WarpComp"
        );
    }

    #[test]
    fn too_close_to_warp_falls_back_to_approach_and_auto_jumps_on_arrival() {
        let mut node = mem_node();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        let near_gate_abs = [gate.abs_m[0] - 2_500.0, gate.abs_m[1], gate.abs_m[2]];
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, near_gate_abs);
        assert!(
            !node.can_propose_jump(ship, JumpGateId(0)),
            "fixture starts outside activation radius"
        );
        assert!(
            !node.can_propose_warp(ship, WarpTarget::Gate(JumpGateId(0))),
            "fixture starts too close to warp"
        );

        let outcome = node.apply_jump_with_fallback(ship, JumpGateId(0));
        assert_eq!(outcome, JumpOutcome::ApproachFallbackStarted);
        assert_eq!(
            node.approach_target(ship),
            Some(dawn_core::ApproachTarget::Gate(JumpGateId(0)))
        );

        let mut pending = Vec::new();
        for _ in 0..400 {
            node.tick();
            pending = node.drain_pending_auto_jumps();
            if !pending.is_empty() {
                break;
            }
        }
        assert_eq!(
            pending,
            vec![(ship, JumpGateId(0))],
            "approach fallback must hand off to the existing auto-jump path on arrival"
        );
    }

    #[test]
    fn unknown_ship_is_rejected() {
        let mut node = mem_node();
        let outcome = node.apply_jump_with_fallback(ShipId::new(NodeId(0), 999), JumpGateId(0));
        assert_eq!(outcome, JumpOutcome::Rejected);
    }

    #[test]
    fn gate_not_in_this_sector_is_rejected() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let outcome = node.apply_jump_with_fallback(ship, JumpGateId(1));
        assert_eq!(outcome, JumpOutcome::Rejected);
    }

    // ── resolve_auto_jump ────────────────────────────────────────────────────

    #[test]
    fn in_range_ship_resolves_auto_jump() {
        let mut node = mem_node();
        let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
        let near_gate_abs = [
            gate.abs_m[0] - (gate.activation_radius as f64 * 0.5),
            gate.abs_m[1],
            gate.abs_m[2],
        ];
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.set_spawn_anchor_abs(ship, near_gate_abs);

        assert_eq!(
            node.resolve_auto_jump(ship, JumpGateId(0)),
            Some(gate.to_sector),
        );
    }

    #[test]
    fn out_of_range_ship_returns_none_for_auto_jump() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(
            !node.can_propose_jump(ship, JumpGateId(0)),
            "fixture starts far from the gate"
        );

        assert_eq!(node.resolve_auto_jump(ship, JumpGateId(0)), None);
    }

    #[test]
    fn unknown_ship_returns_none_for_auto_jump() {
        let node = mem_node();
        assert_eq!(
            node.resolve_auto_jump(ShipId::new(NodeId(0), 999), JumpGateId(0)),
            None,
        );
    }
}
