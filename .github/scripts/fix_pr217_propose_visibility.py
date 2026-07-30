from pathlib import Path

path = Path("crates/dawn-sector/src/node/transit.rs")
text = path.read_text(encoding="utf-8")
old = "    fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {"
new = "    pub(super) fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {"
old_count = text.count(old)
new_count = text.count(new)
if old_count == 1 and new_count == 0:
    path.write_text(text.replace(old, new, 1), encoding="utf-8")
elif old_count == 0 and new_count == 1:
    pass
else:
    raise SystemExit(
        f"unexpected propose_transit visibility state: old={old_count}, new={new_count}"
    )
