from pathlib import Path

path = Path("crates/dawn-client-gdext/src/owned_ship_row_gd.rs")
text = path.read_text()
old = "ship_type_name: row.ship_type_name.unwrap_or_default().into(),"
new = "ship_type_name: row.ship_type_name.as_deref().unwrap_or_default().into(),"
if text.count(old) != 1:
    raise SystemExit(f"expected one GString conversion, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))
