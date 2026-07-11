## client_command_gd_test.gd
##
## Tests for the ClientCommand GDExtension class (dawn-wire/dawn-client-gdext,
## ADR-0041). Each method builds a client -> server wire JSON line; these
## tests parse the returned string back with Godot's own JSON parser and
## check the fields a real server would read, proving the shape matches
## docs/architecture/wire-protocol-commands.schema.json without needing a
## live connection. ClientCommand is a globally registered GDExtension class
## (no preload needed, same as PlayerLoadout/ModuleRow/ItemRow).
extends GdUnitTestSuite

var _cmd: ClientCommand = ClientCommand.new()


func test_move_command_carries_the_target_coordinates() -> void:
	var line: String = _cmd.move_command(10.0, 0.0, -5.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("MoveCommand")
	var target: Dictionary = d["target"]
	assert_float(target["x"]).is_equal_approx(10.0, 0.0001)
	assert_float(target["y"]).is_equal_approx(0.0, 0.0001)
	assert_float(target["z"]).is_equal_approx(-5.0, 0.0001)


## `Option::None` serializes as an explicit JSON `null` (not an omitted key)
## -- the server's Deserialize accepts both identically for Option<T>
## fields, so this is a wire-compatible, if slightly more verbose, shape.
func test_activate_module_command_omits_target_ship_id_when_negative() -> void:
	var line: String = _cmd.activate_module_command(3, "High", -1)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("ActivateModuleCommand")
	assert_int(int(d["module_id"])).is_equal(3)
	assert_str(d["slot"]).is_equal("High")
	assert_bool(d.has("target_ship_id")).is_true()
	assert_object(d["target_ship_id"]).is_null()


func test_activate_module_command_includes_target_ship_id_when_present() -> void:
	var line: String = _cmd.activate_module_command(3, "High", 9)
	var d: Dictionary = JSON.parse_string(line)
	assert_int(int(d["target_ship_id"])).is_equal(9)


func test_orbit_command_omits_radius_when_not_positive() -> void:
	var line: String = _cmd.orbit_command(7, -1.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("OrbitCommand")
	assert_int(int(d["target_id"])).is_equal(7)
	assert_object(d["radius"]).is_null()


func test_orbit_command_includes_radius_when_positive() -> void:
	var line: String = _cmd.orbit_command(7, 2500.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_float(d["radius"]).is_equal_approx(2500.0, 0.0001)


func test_keep_at_range_gate_command_uses_gate_id_not_target_id() -> void:
	var line: String = _cmd.keep_at_range_gate_command(4, 1000.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("KeepAtRangeCommand")
	assert_int(int(d["gate_id"])).is_equal(4)
	assert_object(d["target_id"]).is_null()
	assert_float(d["range"]).is_equal_approx(1000.0, 0.0001)


func test_warp_command_wraps_the_gate_id_in_the_target_tag() -> void:
	var line: String = _cmd.warp_command(2)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("WarpCommand")
	var target: Dictionary = d["target"]
	assert_int(int(target["Gate"])).is_equal(2)


func test_warp_to_body_command_wraps_the_body_id_in_the_target_tag() -> void:
	var line: String = _cmd.warp_to_body_command(5)
	var d: Dictionary = JSON.parse_string(line)
	var target: Dictionary = d["target"]
	assert_int(int(target["Body"])).is_equal(5)


func test_transfer_to_station_command_sets_to_station_direction() -> void:
	var line: String = _cmd.transfer_to_station_command(1, 2, "ScrapMetal", 0, 0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("TransferToStationCommand")
	assert_str(d["direction"]).is_equal("ToStation")
	assert_str(d["item_type"]).is_equal("ScrapMetal")


func test_transfer_from_station_command_sets_to_ship_direction() -> void:
	var line: String = _cmd.transfer_from_station_command(1, 2, "Module", 5, 0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["direction"]).is_equal("ToShip")
	assert_int(int(d["module_id"])).is_equal(5)


func test_undock_command_has_no_extra_fields() -> void:
	var line: String = _cmd.build("UndockCommand", {})
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("UndockCommand")
	assert_int(d.size()).is_equal(1)


## Contract tests for the schema-driven `build()` seam (added when the 14
## flat-scalar commands were collapsed out of individual #[func] wrappers --
## see ADR-0041's follow-up note). These exercise `build()` itself rather
## than one specific command, since all 14 collapsed commands share this one
## mechanism.
func test_build_produces_the_tagged_json_line_for_a_simple_command() -> void:
	var line: String = _cmd.build("DockCommand", {"station_id": 7})
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("DockCommand")
	assert_int(int(d["station_id"])).is_equal(7)


func test_build_returns_empty_string_for_an_unknown_field_name() -> void:
	var line: String = _cmd.build("DockCommand", {"statoin_id": 7})
	assert_str(line).is_equal("")


func test_build_returns_empty_string_for_a_missing_required_field() -> void:
	var line: String = _cmd.build("DockCommand", {})
	assert_str(line).is_equal("")
