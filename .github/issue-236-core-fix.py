from pathlib import Path

path = Path('.github/issue-236-core.py')
text = path.read_text()
old = '''exact(
    "crates/dawn-sector/src/transit.rs",
    "            ship: Box::new(proposal.ship),\\n",
    "            handoff: Box::new(proposal.handoff),\\n",
)
'''
new = '''path = "crates/dawn-sector/src/transit.rs"
text = read(path)
old = "            ship: Box::new(proposal.ship),\\n"
new = "            handoff: Box::new(proposal.handoff),\\n"
index = text.index(old)
write(path, text[:index] + new + text[index + len(old):])
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f'expected one core-script replacement site, found {count}')
path.write_text(text.replace(old, new))
