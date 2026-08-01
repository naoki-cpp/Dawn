from pathlib import Path

path = Path('.github/issue-236-core.py')
text = path.read_text()
old = '''exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "    ship: &ShipSnapshot,\\n",
    "    handoff: &TransitHandoffState,\\n",
)
'''
new = '''exact(
    "crates/dawn-sector/src/transit/pipeline.rs",
    "    ship: &ShipSnapshot,\\n",
    "    handoff: &TransitHandoffState,\\n",
    expected=2,
)
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f'expected one pipeline core-script replacement site, found {count}')
path.write_text(text.replace(old, new))
