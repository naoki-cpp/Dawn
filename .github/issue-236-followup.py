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
    'pending.remove(&event.ship_id);',
    'pending.remove(&event.handoff.ship_id);',
)
exact(
    'crates/dawn-sector/src/transit/pipeline.rs',
    '&& event.ship_id == ship_id',
    '&& event.handoff.ship_id == ship_id',
)
