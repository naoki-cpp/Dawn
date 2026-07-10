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


func test_activate_module_command_omits_target_ship_id_when_negative() -> void:
	var line: String = _cmd.activate_module_command(3, "High", -1)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("ActivateModuleCommand")
	assert_int(d["module_id"]).is_equal(3)
	assert_str(d["slot"]).is_equal("High")
	assert_bool(d.has("target_ship_id")).is_false()


func test_activate_module_command_includes_target_ship_id_when_present() -> void:
	var line: String = _cmd.activate_module_command(3, "High", 9)
	var d: Dictionary = JSON.parse_string(line)
	assert_int(d["target_ship_id"]).is_equal(9)


func test_orbit_command_omits_radius_when_not_positive() -> void:
	var line: String = _cmd.orbit_command(7, -1.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("OrbitCommand")
	assert_int(d["target_id"]).is_equal(7)
	assert_bool(d.has("radius")).is_false()


func test_orbit_command_includes_radius_when_positive() -> void:
	var line: String = _cmd.orbit_command(7, 2500.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_float(d["radius"]).is_equal_approx(2500.0, 0.0001)


func test_keep_at_range_gate_command_uses_gate_id_not_target_id() -> void:
	var line: String = _cmd.keep_at_range_gate_command(4, 1000.0)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("KeepAtRangeCommand")
	assert_int(d["gate_id"]).is_equal(4)
	assert_bool(d.has("target_id")).is_false()
	assert_float(d["range"]).is_equal_approx(1000.0, 0.0001)


func test_warp_command_wraps_the_gate_id_in_the_target_tag() -> void:
	var line: String = _cmd.warp_command(2)
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("WarpCommand")
	var target: Dictionary = d["target"]
	assert_int(target["Gate"]).is_equal(2)


func test_warp_to_body_command_wraps_the_body_id_in_the_target_tag() -> void:
	var line: String = _cmd.warp_to_body_command(5)
	var d: Dictionary = JSON.parse_string(line)
	var target: Dictionary = d["target"]
	assert_int(target["Body"]).is_equal(5)


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
	assert_int(d["module_id"]).is_equal(5)


func test_undock_command_has_no_extra_fields() -> void:
	var line: String = _cmd.undock_command()
	var d: Dictionary = JSON.parse_string(line)
	assert_str(d["type"]).is_equal("UndockCommand")
	assert_int(d.size()).is_equal(1)
