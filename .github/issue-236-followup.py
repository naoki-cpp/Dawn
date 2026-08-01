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
