from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}")
    file_path.write_text(text.replace(old, new, 1))


path = "client/test/main_test.gd"

replace_once(
    path,
    '''

class TestableMain:
\textends MainScript
''',
    '''

class FakeWorldPresentation:
\textends WorldPresentation

\tfunc attach_player_ship(ship: Node3D, _weapon_range: float, _weapon_falloff: float) -> void:
\t\tif ship == null:
\t\t\treturn
\t\tif _player_ship != ship:
\t\t\tdetach_player_ship()
\t\t_player_ship = ship
\t\tship.call("set_as_player")

\tfunc detach_player_ship() -> void:
\t\tif _player_ship != null and is_instance_valid(_player_ship):
\t\t\t_player_ship.call("clear_as_player")
\t\t_player_ship = null

\tfunc update_tactical_overlay_ranges(_weapon_range: float, _weapon_falloff: float) -> void:
\t\tpass


class TestableMain:
\textends MainScript
''',
)

replace_once(
    path,
    '''func _replace_with_testable_main() -> void:
\t_main.free()
\t_main = TestableMain.new()
\t_initialize_main_dependencies()
''',
    '''func _replace_with_testable_main() -> void:
\t_main.free()
\t_main = TestableMain.new()
\t_main._presentation = FakeWorldPresentation.new()
\t_initialize_main_dependencies()
''',
)

replace_once(
    path,
    '''\t_dispatch_fixture("PlayerLoadoutUnknownDocked", 11)
\t_main._apply_loadout_side_effects()
\tassert_int(_main._session.player_ship_id()).is_equal(11)
''',
    '''\t_dispatch_fixture("PlayerLoadoutUnknownDocked", 11)
\t_main._apply_current_dock_state_to_player_ship(old_ship)
\tassert_int(_main._session.player_ship_id()).is_equal(11)
''',
)

print("pending dock GdUnit fixture fixed")
