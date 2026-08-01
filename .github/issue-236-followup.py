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
