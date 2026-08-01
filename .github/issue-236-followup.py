from pathlib import Path


def exact(path, old, new, expected=1):
    p = Path(path)
    text = p.read_text()
    count = text.count(old)
    if count != expected:
        raise SystemExit(f'{path}: expected {expected}, found {count}: {old!r}')
    p.write_text(text.replace(old, new))


exact(
    'crates/dawn-sector/src/transit/pipeline.rs',
    '''            DomainEvent::SectorTransitCompleted(event) if event.from == sector_id => {
                pending.remove(&event.ship_id);
            }
''',
    '''            DomainEvent::SectorTransitCompleted(event) if event.from == sector_id => {
                pending.remove(&event.handoff.ship_id);
            }
''',
)
exact(
    'crates/dawn-sector/src/transit/pipeline.rs',
    '''            DomainEvent::SectorTransitCompleted(event)
                if marker_seen
                    && event.ship_id == ship_id
''',
    '''            DomainEvent::SectorTransitCompleted(event)
                if marker_seen
                    && event.handoff.ship_id == ship_id
''',
)
exact(
    'crates/dawn-wire/src/server_event.rs',
    '''            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                ship_id: ship_id(1),
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                velocity: dawn_core::Velocity::ZERO,
                tick,
                ship_state: dawn_core::events::TransitShipState {
                    ship_type_id: dawn_core::ShipTypeId(1),
                    current_shield: 100.0,
                    current_armor: 100.0,
                    current_hull: 100.0,
                    is_destroyed: false,
                    capacitor: Some(50.0),
                    fitting: dawn_core::fitting::FittingSnapshot::empty(),
                    inventory: std::collections::BTreeMap::new(),
                },
            }),
''',
    '''            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                handoff: dawn_core::TransitHandoffState {
                    ship_id: ship_id(1),
                    ship_type_id: dawn_core::ShipTypeId(1),
                    velocity: dawn_core::Velocity::ZERO,
                    current_shield: 100.0,
                    current_armor: 100.0,
                    current_hull: 100.0,
                    is_destroyed: false,
                    capacitor: Some(50.0),
                    fitting: dawn_core::fitting::FittingSnapshot::empty(),
                    inventory: std::collections::BTreeMap::new(),
                },
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                tick,
            }),
''',
)
exact(
    'crates/dawn-sector/src/node/transit.rs',
    '''    /// Takes `ship` (the same `ShipSnapshot` the `TransitOp::Commit` payload
    /// carries, echoed back to this Sector along with everyone else's copy)
    /// rather than re-reading the Ship's current ECS state: the Ship has been
    /// frozen out of Movement/Combat since Request-commit
    /// (`dawn-ecs`'s `TransitComp` guards), so nothing should have changed it
    /// in the meantime, and using the one payload both `from` and `to` share
    /// keeps their `SectorTransitCompleted.ship_state` identical by
    /// construction instead of by coincidence.
''',
    '''    /// Ack carries only the transfer identity. The source re-reads the
    /// canonical handoff state from its frozen recovery copy before removal;
    /// `TransitComp` guards guarantee that state has not changed since Request.
    /// The resulting `SectorTransitCompleted.handoff` therefore matches the
    /// state previously proposed to the destination without coupling Ack to a
    /// second copy of the Ship payload.
''',
)
exact(
    'crates/dawn-sector/src/node/transit.rs',
    '''    /// The `to` branch does not call `restore_ship_from_snapshot` through
    /// `import_transit` (which also appends events) -- replay must not
    /// append anything it didn't already record, so it rebuilds a
    /// `ShipSnapshot` from `e.ship_state` via `ship_snapshot_from_transit`
    /// and redoes the anchor rebase state directly via
    /// `rebase_ship_anchor_state` (see that method's doc comment for why the
    /// already-logged `AnchorRebased` entry can't do this on its own).
''',
    '''    /// The `to` branch feeds `e.handoff` through the same direct
    /// handoff-to-ECS mapping as live import, without appending new events.
    /// It then redoes the anchor rebase state directly via
    /// `rebase_ship_anchor_state` (see that method's doc comment for why the
    /// already-logged `AnchorRebased` entry can't do this on its own).
''',
)
