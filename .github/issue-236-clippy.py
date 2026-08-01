from pathlib import Path

path = Path('crates/dawn-sector/src/transit/pipeline.rs')
text = path.read_text()
old = '''    if node.get_ship_position(ship_id).is_none() {
        return None;
    }
    pending_outgoing_transits(node).into_iter().find(|pending| {
'''
new = '''    node.get_ship_position(ship_id)?;
    pending_outgoing_transits(node).into_iter().find(|pending| {
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f'expected one matching_request guard, found {count}')
path.write_text(text.replace(old, new))
