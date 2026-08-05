#!/usr/bin/env python3
"""Restore the original Transit tests and add review regressions."""

from __future__ import annotations

import subprocess
from pathlib import Path

PATH = Path("crates/dawn-sector/src/node/transit.rs")
MARKER = "#[cfg(test)]\nmod tests {"


def main() -> None:
    current = PATH.read_text()
    main_source = subprocess.check_output(
        ["git", "show", f"origin/main:{PATH.as_posix()}"],
        text=True,
    )

    current_impl = current[: current.index(MARKER)]
    tests = main_source[main_source.index(MARKER) :]

    atomic_test = r'''

    #[test]
    fn prepare_transit_commit_rolls_back_when_handoff_snapshot_is_incomplete() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let _ = node.world.remove_one::<VelocityComp>(entity).unwrap();
        let event_count = node.total_event_count();

        assert!(node
            .prepare_transit_commit(ship_id, SectorId(1), None)
            .is_none());
        assert_eq!(node.world.transit_state(entity), TransitState::None);
        assert_eq!(node.total_event_count(), event_count);
        assert!(node.can_propose_transit(ship_id));
        assert!(!node.event_store().all_records().iter().any(|record| {
            matches!(record.event, DomainEvent::SectorTransitRequested(_))
        }));
    }
'''
    atomic_anchor = (
        "\n    #[test]\n"
        "    fn propose_transit_is_rejected_when_ship_is_already_in_transit()"
    )
    if atomic_anchor not in tests:
        raise RuntimeError("atomic test insertion anchor not found")
    tests = tests.replace(atomic_anchor, atomic_test + atomic_anchor, 1)

    stale_abort_test = r'''

    #[test]
    fn replaying_stale_abort_for_an_old_route_keeps_the_current_marker() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        for event in [
            DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                request_tick: Tick(1),
                gate_id: None,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(1),
            }),
            DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
                ship_id,
                from: SectorId(0),
                to: SectorId(2),
                request_tick: Tick(2),
                gate_id: None,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                tick: Tick(2),
            }),
            DomainEvent::SectorTransitAborted(dawn_core::events::SectorTransitAborted {
                ship_id,
                from: SectorId(0),
                to: SectorId(1),
                tick: Tick(3),
            }),
        ] {
            node.apply_event_pub(event);
        }

        let entity = *node.ships.index.get(&ship_id).unwrap();
        assert_eq!(
            node.world.transit_state(entity),
            TransitState::InTransit { to: SectorId(2) }
        );
    }
'''
    abort_anchor = (
        "\n    #[test]\n"
        "    fn replaying_completed_on_the_source_sector_removes_the_ship()"
    )
    if abort_anchor not in tests:
        raise RuntimeError("abort test insertion anchor not found")
    tests = tests.replace(abort_anchor, stale_abort_test + abort_anchor, 1)

    PATH.write_text(current_impl + tests)


if __name__ == "__main__":
    main()
