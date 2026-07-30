from pathlib import Path


for name in (
    "crates/dawn-sector/src/node/mod.rs",
    "crates/dawn-sector/src/node/snapshot_io.rs",
):
    path = Path(name)
    text = path.read_text().replace("std::collections::Vec", "Vec")
    path.write_text(text)

path = Path("crates/dawn-sector/src/node/transit_flow.rs")
text = path.read_text()
old = """            Some(outbound_gate.id),
        );"""
new = """            Some(outbound_gate.id),
            data.request_tick,
        );"""
if text.count(old) != 1:
    raise SystemExit(
        f"direct commit test call: expected one match, found {text.count(old)}"
    )
path.write_text(text.replace(old, new, 1))
