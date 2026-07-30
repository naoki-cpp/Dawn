from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    p.write_text(text.replace(old, new, 1))


def replace_span(path: str, start: str, end: str, replacement: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    start_i = text.find(start)
    if start_i < 0:
        raise SystemExit(f"{label}: start marker not found")
    end_i = text.find(end, start_i + len(start))
    if end_i < 0:
        raise SystemExit(f"{label}: end marker not found")
    p.write_text(text[:start_i] + replacement + text[end_i:])


replace_once(
    "crates/dawn-client-gdext/src/lib.rs",
    "mod navigation_gd;\n",
    "mod navigation_gd;\nmod owned_ship_row_gd;\n",
    "gdext module registration",
)
replace_once(
    "crates/dawn-client-gdext/src/lib.rs",
    "pub use module_row_gd::ModuleRow;\n",
    "pub use module_row_gd::ModuleRow;\npub use owned_ship_row_gd::OwnedShipRow;\n",
    "gdext type export",
)

replace_span(
    "client/scripts/main.gd",
    "\tvar dock_status: Dictionary = _loadout.dock_status()\n",
    "\t_sync_session_state()\n",
    '''\t_session.apply_dock_fitting(
\t\t_loadout.docked_station_id(),
\t\t_loadout.docked_station_name(),
\t\t_loadout.tick()
\t)
''',
    "dock scalar reads",
)
replace_span(
    "client/scripts/main.gd",
    "\tvar snapshot: Dictionary = _loadout.hud_snapshot()\n",
    "\t_recalc_weapon_range()\n",
    '''\tvar modules := _loadout.modules()
\tvar inventory := _loadout.inventory()
\tvar station_inventory := _loadout.station_inventory()
\tvar owned_ships := _loadout.owned_ships()
\t_hud_surface.set_player_fitting(
\t\tmodules, inventory, station_inventory, owned_ships, _buildable_ship_types)
\t_market_surface.set_cargo(inventory)
''',
    "direct fitting reads",
)
replace_span(
    "client/scripts/main.gd",
    "func _recalc_weapon_range() -> void:\n",
    "func _on_module_activated",
    '''func _recalc_weapon_range() -> void:
\t_weapon_range = _loadout.weapon_optimal_range()
\t_weapon_falloff = _loadout.weapon_falloff_range()
\t_presentation.update_tactical_overlay_ranges(_weapon_range, _weapon_falloff)

''',
    "weapon scalar reads",
)
replace_span(
    "client/scripts/main.gd",
    "\t\t\tvar snapshot: Dictionary = _loadout.hud_snapshot()\n",
    "\t\tInventoryRow.ACTION_BUILD_SHIP_TYPE:",
    '''\t\t\t_hud_surface.toggle_build_picker(
\t\t\t\t_loadout.modules(),
\t\t\t\t_loadout.inventory(),
\t\t\t\t_loadout.station_inventory(),
\t\t\t\t_loadout.owned_ships(),
\t\t\t\t_buildable_ship_types)
''',
    "build picker reads",
)
