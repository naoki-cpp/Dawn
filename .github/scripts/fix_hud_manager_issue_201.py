from pathlib import Path
import subprocess

base_sha = "d9846778abc48fe0395f338fc1d6efc12192a914"
path = "client/scripts/hud_manager.gd"
text = subprocess.check_output(
    ["git", "show", f"{base_sha}:{path}"],
    text=True,
)

old_comment = """## ModuleRow/ItemRow are GDExtension classes (dawn-client-gdext,
## ADR-0039/ADR-0040) -- globally registered, no preload needed."""
new_comment = """## ModuleRow/ItemRow/OwnedShipRow are GDExtension classes
## (dawn-client-gdext, ADR-0039/ADR-0040) -- globally registered, no preload needed."""
if text.count(old_comment) != 1:
    raise SystemExit(f"expected one class comment, found {text.count(old_comment)}")
text = text.replace(old_comment, new_comment, 1)

old_loop = '''\tvar ship_rows: Array[InventoryRow] = []
\tfor entry: Variant in owned_ships:
\t\tvar ship: Dictionary = entry as Dictionary
\t\tvar ship_id: int = ship.get("ship_id", 0) as int
\t\tvar is_active: bool = ship.get("is_active", false) as bool
\t\tvar raw_ship_type_name: Variant = ship.get("ship_type_name", null)
\t\tvar ship_type_name: String = "" if raw_ship_type_name == null else raw_ship_type_name as String
\t\tvar name := ship_type_name if not ship_type_name.is_empty() else "Ship #%d" % ship_id
\t\tvar raw_docked_station_id: Variant = ship.get("docked_station_id", null)
\t\tvar docked_station_id: int = -1 if raw_docked_station_id == null else raw_docked_station_id as int
\t\tvar status := "active" if is_active else ("docked" if docked_station_id >= 0 else "away")
'''
new_loop = '''\tvar ship_rows: Array[InventoryRow] = []
\tfor entry: Variant in owned_ships:
\t\tvar ship: OwnedShipRow = entry as OwnedShipRow
\t\tvar ship_id: int = ship.ship_id
\t\tvar is_active: bool = ship.is_active
\t\tvar name := ship.ship_type_name if not ship.ship_type_name.is_empty() else "Ship #%d" % ship_id
\t\tvar status := "active" if is_active else ("docked" if ship.docked_station_id >= 0 else "away")
'''
if text.count(old_loop) != 1:
    raise SystemExit(f"expected one owned-ship loop, found {text.count(old_loop)}")
text = text.replace(old_loop, new_loop, 1)
Path(path).write_text(text)
