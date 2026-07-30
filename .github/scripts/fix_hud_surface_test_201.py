from pathlib import Path

path = Path("client/test/hud_surface_test.gd")
text = path.read_text()

marker = '''func _item(overrides: Dictionary) -> ItemRow:
\tvar base: Dictionary = {
\t\t"item_type": "Module", "module_id": 1, "ship_type_id": 0,
\t\t"name": "Test Item", "kind": "", "slot": "", "count": 1,
\t}
\tfor key: String in overrides:
\t\tbase[key] = overrides[key]
\treturn ItemRow.from_json(base)
'''
helper = marker + '''

func _owned_ship(overrides: Dictionary) -> OwnedShipRow:
\tvar base: Dictionary = {
\t\t"ship_id": 1,
\t\t"ship_type_id": 7,
\t\t"ship_type_name": "Magpie",
\t\t"docked_station_id": 0,
\t\t"is_active": true,
\t}
\tfor key: String in overrides:
\t\tbase[key] = overrides[key]
\treturn OwnedShipRow.from_json(base)
'''
if text.count(marker) != 1:
    raise SystemExit(f"expected one item helper, found {text.count(marker)}")
text = text.replace(marker, helper, 1)

old = '''\t_surface.set_player_fitting([], [], [], [
\t\t{"ship_id": 1, "ship_type_id": 7, "ship_type_name": "Magpie", "docked_station_id": 0, "is_active": true},
\t\t{"ship_id": 2, "ship_type_id": 7, "ship_type_name": "Magpie", "docked_station_id": 0, "is_active": false},
\t])
'''
new = '''\t_surface.set_player_fitting([], [], [], [
\t\t_owned_ship({}),
\t\t_owned_ship({"ship_id": 2, "is_active": false}),
\t])
'''
if text.count(old) != 1:
    raise SystemExit(f"expected one owned roster fixture, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))
