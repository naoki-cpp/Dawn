from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}")
    file_path.write_text(text.replace(old, new, 1))


# Reapply the authoritative dock projection whenever a ship becomes the
# presentation's player ship, including delayed AoiEnter/ShipSpawned promotion.
replace_once(
    "client/scripts/main.gd",
    '''func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
\t_player_ship_id = p_ship_id
\t_presentation.attach_player_ship(ship, _weapon_range, _weapon_falloff)
''',
    '''func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
\t_player_ship_id = p_ship_id
\t_presentation.attach_player_ship(ship, _weapon_range, _weapon_falloff)
\t_apply_current_dock_state_to_player_ship(ship)


func _apply_current_dock_state_to_player_ship(ship: Node3D) -> void:
\tif ship == null or not _session.is_docked():
\t\treturn
\tship.call(
\t\t"dock_motion",
\t\tship.call("server_position"),
\t\t_session.latest_dock_state_tick())
''',
)

replace_once(
    "client/scripts/main.gd",
    '''func _apply_loadout_side_effects() -> void:
\tvar new_active_ship_id: int = _session.player_ship_id()
\tif new_active_ship_id != _player_ship_id:
\t\tif new_active_ship_id >= 0 and _ships.has(new_active_ship_id):
\t\t\t_set_as_player_ship(new_active_ship_id, _ships[new_active_ship_id] as Node3D)
\t\telif new_active_ship_id < 0:
\t\t\t_player_ship_id = new_active_ship_id
\t\t\t_presentation.detach_player_ship()
\t_sync_session_state()
''',
    '''func _apply_loadout_side_effects() -> void:
\tvar new_active_ship_id: int = _session.player_ship_id()
\tvar attached_new_player := false
\tif new_active_ship_id != _player_ship_id:
\t\tif new_active_ship_id >= 0 and _ships.has(new_active_ship_id):
\t\t\t_set_as_player_ship(new_active_ship_id, _ships[new_active_ship_id] as Node3D)
\t\t\tattached_new_player = true
\t\telif new_active_ship_id < 0:
\t\t\t_player_ship_id = new_active_ship_id
\t\t\t_presentation.detach_player_ship()
\t_sync_session_state()
''',
)

replace_once(
    "client/scripts/main.gd",
    '''\tif _session.is_docked() and _player_ship_id >= 0:
\t\tvar docked_ship := _ships.get(_player_ship_id) as Node3D
\t\tif docked_ship != null:
\t\t\tdocked_ship.call("dock_motion", docked_ship.call("server_position"), _loadout.tick())
''',
    '''\tif not attached_new_player and _player_ship_id >= 0:
\t\t_apply_current_dock_state_to_player_ship(
\t\t\t_ships.get(_player_ship_id) as Node3D)
''',
)

# Add debug fixtures that exercise the real inbound AoiEnter and ShipSpawned
# routes after an unknown-but-docked PlayerLoadout switch.
replace_once(
    "crates/dawn-client-gdext/src/server_message_gd.rs",
    '''        other_ship.cap_max = 80.0;
        other_ship.cap_recharge_per_tick = 4.0;

        let loadout = |tick, active_ship_id| PlayerLoadoutWire {
''',
    '''        other_ship.cap_max = 80.0;
        other_ship.cap_recharge_per_tick = 4.0;
        let mut pending_ship = other_ship.clone();
        pending_ship.ship_id = 33;
        pending_ship.ship_type_name = "Prospect".to_owned();

        let loadout = |tick, active_ship_id| PlayerLoadoutWire {
''',
)

replace_once(
    "crates/dawn-client-gdext/src/server_message_gd.rs",
    '''            active_ship_id,
            owned_ships: Vec::new(),
        };

        let message = match kind.to_string().as_str() {
''',
    '''            active_ship_id,
            owned_ships: Vec::new(),
        };
        let docked_loadout = |tick, active_ship_id| {
            let mut result = loadout(tick, active_ship_id);
            result.docked_station_id = Some(5);
            result.docked_station_name = Some("Forge Station".to_owned());
            result
        };

        let message = match kind.to_string().as_str() {
''',
)

replace_once(
    "crates/dawn-client-gdext/src/server_message_gd.rs",
    '''            "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
            "InitialState" => ServerMessage::InitialState(InitialStateWire {
''',
    '''            "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
            "AoiEnterPending" => ServerMessage::AoiEnter(pending_ship.clone()),
            "InitialState" => ServerMessage::InitialState(InitialStateWire {
''',
)

replace_once(
    "crates/dawn-client-gdext/src/server_message_gd.rs",
    '''            "PlayerLoadoutSwitch" => ServerMessage::PlayerLoadout(loadout(12, Some(22))),
            "PlayerLoadoutUnknown" => ServerMessage::PlayerLoadout(loadout(13, Some(33))),
            "PlayerLoadoutDisembark" => ServerMessage::PlayerLoadout(loadout(14, None)),
''',
    '''            "PlayerLoadoutSwitch" => ServerMessage::PlayerLoadout(loadout(12, Some(22))),
            "PlayerLoadoutUnknown" => ServerMessage::PlayerLoadout(loadout(13, Some(33))),
            "PlayerLoadoutUnknownDocked" => {
                ServerMessage::PlayerLoadout(docked_loadout(13, Some(33)))
            }
            "PlayerLoadoutDisembark" => ServerMessage::PlayerLoadout(loadout(14, None)),
''',
)

replace_once(
    "crates/dawn-client-gdext/src/server_message_gd.rs",
    '''            "ShipDocked" => ServerMessage::Event(EventWire::ShipDocked {
                ship_id: 11,
                station_id: 5,
                tick: 12,
            }),
            "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
''',
    '''            "ShipDocked" => ServerMessage::Event(EventWire::ShipDocked {
                ship_id: 11,
                station_id: 5,
                tick: 12,
            }),
            "ShipSpawnedPending" => ServerMessage::Event(EventWire::ShipSpawned {
                ship_id: 33,
                position,
                tick: 14,
            }),
            "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
''',
)

# Extend the main GdUnit harness with a fake ship factory and end-to-end
# pending-promotion regressions for both registration message families.
replace_once(
    "client/test/main_test.gd",
    '''const __source: String = "res://scripts/main.gd"
const InventoryRow = preload("res://scripts/inventory_row.gd")
''',
    '''const __source: String = "res://scripts/main.gd"
const MainScript = preload("res://scripts/main.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")
''',
)

replace_once(
    "client/test/main_test.gd",
    '''func _dispatch_fixture(kind: String, connection_ship_id: int = -1) -> void:
\tvar outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
\tassert_object(outcome).is_not_null()
\tvar target := TypedOutcomeTarget.new()
\tassert_bool(outcome.dispatch(
\t\ttarget, target, _main._session, _main._loadout, connection_ship_id
\t)).is_true()


func _setup_docked_session() -> void:
''',
    '''func _dispatch_fixture(kind: String, connection_ship_id: int = -1) -> void:
\tvar outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
\tassert_object(outcome).is_not_null()
\tvar target := TypedOutcomeTarget.new()
\tassert_bool(outcome.dispatch(
\t\ttarget, target, _main._session, _main._loadout, connection_ship_id
\t)).is_true()


func _dispatch_to_main(kind: String, connection_ship_id: int = -1) -> void:
\tvar outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
\tassert_object(outcome).is_not_null()
\tvar connection_target := TypedOutcomeTarget.new()
\tassert_bool(outcome.dispatch(
\t\tconnection_target, _main, _main._session, _main._loadout, connection_ship_id
\t)).is_true()


func _setup_docked_session() -> void:
''',
)

replace_once(
    "client/test/main_test.gd",
    '''\tfunc set_velocity(v: Vector3, tick: int = 0) -> bool:
''',
    '''\tfunc configure_motion(
\t\t_max_speed: float,
\t\t_mass: float,
\t\t_inertia_modifier: float,
\t\tp: PackedFloat64Array,
\t\tv: Vector3,
\t\t_tick: int = 0
\t) -> void:
\t\tserver_position_value = p
\t\tvelocity_calls.append(v)

\tfunc set_velocity(v: Vector3, tick: int = 0) -> bool:
''',
)

replace_once(
    "client/test/main_test.gd",
    '''

class FakeConnection:
''',
    '''

class TestableMain:
\textends MainScript

\tvar instantiated_ships: Array[Node3D] = []

\tfunc _instantiate_ship(sid: int, server_pos: PackedFloat64Array) -> Node3D:
\t\tvar ship := FakeShip.new()
\t\tship.name = "Ship_%d" % sid
\t\tship.server_position_value = server_pos
\t\tadd_child(ship)
\t\tinstantiated_ships.append(ship)
\t\treturn ship


class FakeConnection:
''',
)

replace_once(
    "client/test/main_test.gd",
    '''func before_test() -> void:
\t## .new() without adding to the scene tree never triggers _ready(), so
\t## the @onready scene-path vars stay null -- fine, since none of the
\t## functions under test touch them.
\t_main = load(__source).new()
\t## _ready() normally injects WorldSpace through WorldPresentation.build().
\t## This fixture skips _ready(), so establish the same production dependency.
\t_main._presentation._world = _main._world
\t_main._interaction = load("res://scripts/world_interaction.gd").new()
\t_main._loadout = PlayerLoadout.new()


func after_test() -> void:
''',
    '''func before_test() -> void:
\t## .new() without adding to the scene tree never triggers _ready(), so
\t## the @onready scene-path vars stay null -- fine, since none of the
\t## functions under test touch them.
\t_main = load(__source).new()
\t## _ready() normally injects WorldSpace through WorldPresentation.build().
\t## This fixture skips _ready(), so establish the same production dependency.
\t_initialize_main_dependencies()


func _initialize_main_dependencies() -> void:
\t_main._presentation._world = _main._world
\t_main._interaction = load("res://scripts/world_interaction.gd").new()
\t_main._loadout = PlayerLoadout.new()


func _replace_with_testable_main() -> void:
\t_main.free()
\t_main = TestableMain.new()
\t_initialize_main_dependencies()


func after_test() -> void:
''',
)

replace_once(
    "client/test/main_test.gd",
    '''func _set_loadout_modules(modules: Array[ModuleRow]) -> void:
\tvar owned_ships: Array[OwnedShipRow] = []
\tassert_bool(_main._loadout.test_fixture(
\t\t0, modules, -1, "", -1, owned_ships
\t)).is_true()




func test_warp_hud_guidance_uses_shared_minimum_distance_boundary() -> void:
''',
    '''func _set_loadout_modules(modules: Array[ModuleRow]) -> void:
\tvar owned_ships: Array[OwnedShipRow] = []
\tassert_bool(_main._loadout.test_fixture(
\t\t0, modules, -1, "", -1, owned_ships
\t)).is_true()


func _setup_pending_docked_switch() -> FakeShip:
\t_replace_with_testable_main()
\tvar old_ship := FakeShip.new()
\t_main.add_child(old_ship)
\t_main._ships = {11: old_ship}
\t_dispatch_fixture("InitialState", 11)
\t_main._set_as_player_ship(11, old_ship)
\t_dispatch_fixture("ShipDocked", 11)
\t_dispatch_fixture("PlayerLoadoutUnknownDocked", 11)
\t_main._apply_loadout_side_effects()
\tassert_int(_main._session.player_ship_id()).is_equal(11)
\tassert_bool(_main._session.is_docked()).is_true()
\tassert_int(old_ship.dock_calls.size()).is_equal(1)
\treturn old_ship


func test_warp_hud_guidance_uses_shared_minimum_distance_boundary() -> void:
''',
)

replace_once(
    "client/test/main_test.gd",
    '''

## Disembarking is also applied by the typed PlayerLoadout outcome before the
''',
    '''

func test_pending_docked_switch_reapplies_dock_after_aoi_enter() -> void:
\tvar old_ship := _setup_pending_docked_switch()

\t_dispatch_to_main("AoiEnterPending", 11)

\tassert_int(_main._session.player_ship_id()).is_equal(33)
\tassert_int(_main._player_ship_id).is_equal(33)
\tassert_bool(_main._session.is_docked()).is_true()
\tassert_bool(_main._ships.has(33)).is_true()
\tvar new_ship := _main._ships[33] as FakeShip
\tassert_int(new_ship.set_as_player_calls).is_equal(1)
\tassert_int(new_ship.dock_calls.size()).is_equal(1)
\tassert_int(new_ship.dock_calls[0]["tick"] as int).is_equal(13)
\tassert_int(old_ship.clear_as_player_calls).is_equal(1)


func test_pending_docked_switch_reapplies_dock_after_ship_spawned() -> void:
\tvar old_ship := _setup_pending_docked_switch()

\t_dispatch_to_main("ShipSpawnedPending", 11)

\tassert_int(_main._session.player_ship_id()).is_equal(33)
\tassert_int(_main._player_ship_id).is_equal(33)
\tassert_bool(_main._session.is_docked()).is_true()
\tassert_bool(_main._ships.has(33)).is_true()
\tvar new_ship := _main._ships[33] as FakeShip
\tassert_int(new_ship.set_as_player_calls).is_equal(1)
\tassert_int(new_ship.dock_calls.size()).is_equal(1)
\tassert_int(new_ship.dock_calls[0]["tick"] as int).is_equal(13)
\tassert_int(old_ship.clear_as_player_calls).is_equal(1)


## Disembarking is also applied by the typed PlayerLoadout outcome before the
''',
)

print("pending dock promotion fix applied")
