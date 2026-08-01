## client_command_gd_test.gd
##
## Tests for the ClientCommand GDExtension class (dawn-wire/dawn-client-gdext,
## ADR-0041/ADR-0042). Each method builds a client -> server wire message,
## postcard-encoded as a ClientMessage::Command envelope. These tests decode
## the returned bytes back with ClientMessageDecoder (test-only helper, the
## reverse of what ClientCommand builds) and check the fields a real server
## would read, proving the shape matches
## docs/architecture/wire-protocol-commands.schema.json without needing a
## live connection. ClientCommand/ClientMessageDecoder/ItemIdentity are
## globally registered GDExtension classes.
extends GdUnitTestSuite

var _cmd: ClientCommand = ClientCommand.new()
var _decoder: ClientMessageDecoder = ClientMessageDecoder.new()


func test_move_command_carries_the_target_coordinates() -> void:
	var bytes: PackedByteArray = _cmd.move_command(10.0, 0.0, -5.0)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("MoveCommand")
	var target: Dictionary = d["target"]
	assert_float(target["x"]).is_equal_approx(10.0, 0.0001)
	assert_float(target["y"]).is_equal_approx(0.0, 0.0001)
	assert_float(target["z"]).is_equal_approx(-5.0, 0.0001)


## `Option::None` serializes as an explicit JSON `null` (not an omitted key)
## -- the server's Deserialize accepts both identically for Option<T>
## fields, so this is a wire-compatible, if slightly more verbose, shape.
func test_activate_module_command_omits_target_ship_id_when_negative() -> void:
	var bytes: PackedByteArray = _cmd.activate_module_command(3, "High", -1)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("ActivateModuleCommand")
	assert_int(int(d["module_id"])).is_equal(3)
	assert_str(d["slot"]).is_equal("High")
	assert_bool(d.has("target_ship_id")).is_true()
	assert_object(d["target_ship_id"]).is_null()


func test_activate_module_command_includes_target_ship_id_when_present() -> void:
	var bytes: PackedByteArray = _cmd.activate_module_command(3, "High", 9)
	var d: Dictionary = _decoder.decode(bytes)
	assert_int(int(d["target_ship_id"])).is_equal(9)


func test_approach_command_wraps_the_ship_id_in_the_target_tag() -> void:
	var bytes: PackedByteArray = _cmd.approach_command(7)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("ApproachCommand")
	var target: Dictionary = d["target"]
	assert_int(int(target["Ship"])).is_equal(7)


func test_approach_gate_command_wraps_the_gate_id_in_the_target_tag() -> void:
	var bytes: PackedByteArray = _cmd.approach_gate_command(4)
	var d: Dictionary = _decoder.decode(bytes)
	var target: Dictionary = d["target"]
	assert_int(int(target["Gate"])).is_equal(4)


func test_orbit_command_omits_radius_when_not_positive() -> void:
	var bytes: PackedByteArray = _cmd.orbit_command(7, -1.0)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("OrbitCommand")
	var target: Dictionary = d["target"]
	assert_int(int(target["Ship"])).is_equal(7)
	assert_object(d["radius"]).is_null()


func test_orbit_command_includes_radius_when_positive() -> void:
	var bytes: PackedByteArray = _cmd.orbit_command(7, 2500.0)
	var d: Dictionary = _decoder.decode(bytes)
	assert_float(d["radius"]).is_equal_approx(2500.0, 0.0001)


func test_keep_at_range_gate_command_uses_a_tagged_gate_target() -> void:
	var bytes: PackedByteArray = _cmd.keep_at_range_gate_command(4, 1000.0)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("KeepAtRangeCommand")
	var target: Dictionary = d["target"]
	assert_int(int(target["Gate"])).is_equal(4)
	assert_float(d["range"]).is_equal_approx(1000.0, 0.0001)


func test_warp_command_wraps_the_gate_id_in_the_target_tag() -> void:
	var bytes: PackedByteArray = _cmd.warp_command(2)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("WarpCommand")
	var target: Dictionary = d["target"]
	assert_int(int(target["Gate"])).is_equal(2)


func test_warp_to_body_command_wraps_the_body_id_in_the_target_tag() -> void:
	var bytes: PackedByteArray = _cmd.warp_to_body_command(5)
	var d: Dictionary = _decoder.decode(bytes)
	var target: Dictionary = d["target"]
	assert_int(int(target["Body"])).is_equal(5)


func test_transfer_to_station_command_preserves_scrap_identity() -> void:
	var bytes: PackedByteArray = _cmd.transfer_to_station_command(
		1, 2, ItemIdentity.scrap_metal())
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("TransferToStationCommand")
	assert_str(d["direction"]).is_equal("ToStation")
	assert_str(d["item_id"]).is_equal("ScrapMetal")


func test_transfer_from_station_command_preserves_module_identity() -> void:
	var module_id: ItemIdentity = ItemIdentity.module(5) as ItemIdentity
	var bytes: PackedByteArray = _cmd.transfer_from_station_command(1, 2, module_id)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["direction"]).is_equal("ToShip")
	var item_id: Dictionary = d["item_id"]
	var module: Dictionary = item_id["Module"]
	assert_int(int(module["module_id"])).is_equal(5)


func test_transfer_preserves_packaged_ship_identity() -> void:
	var ship_item: ItemIdentity = ItemIdentity.packaged_ship(7) as ItemIdentity
	var bytes: PackedByteArray = _cmd.transfer_to_station_command(1, 2, ship_item)
	var d: Dictionary = _decoder.decode(bytes)
	var item_id: Dictionary = d["item_id"]
	var packaged_ship: Dictionary = item_id["PackagedShip"]
	assert_int(int(packaged_ship["ship_type_id"])).is_equal(7)


func test_undock_command_has_no_extra_fields() -> void:
	var bytes: PackedByteArray = _cmd.build("UndockCommand", {})
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("UndockCommand")
	assert_int(d.size()).is_equal(1)


## Contract tests for the schema-driven `build()` seam (added when the 14
## flat-scalar commands were collapsed out of individual #[func] wrappers --
## see ADR-0041's follow-up note).
func test_build_produces_the_tagged_message_for_a_simple_command() -> void:
	var bytes: PackedByteArray = _cmd.build("DockCommand", {"station_id": 7})
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("DockCommand")
	assert_int(int(d["station_id"])).is_equal(7)


func test_build_returns_empty_bytes_for_an_unknown_field_name() -> void:
	var bytes: PackedByteArray = _cmd.build("DockCommand", {"statoin_id": 7})
	assert_bool(bytes.is_empty()).is_true()


func test_build_returns_empty_bytes_for_a_missing_required_field() -> void:
	var bytes: PackedByteArray = _cmd.build("DockCommand", {})
	assert_bool(bytes.is_empty()).is_true()


func test_market_command_preserves_the_typed_item_identity() -> void:
	var bytes := _cmd.market_place_order_command(
		42, ItemIdentity.scrap_metal(), "Ask", 100, 3)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("PlaceMarketOrderCommand")
	assert_str(d["side"]).is_equal("Ask")
	assert_int(int(d["ship_id"])).is_equal(42)
	assert_int(int(d["quantity"])).is_equal(3)
	assert_str(d["item_id"]).is_equal("ScrapMetal")


## Hello (ADR-0007/ADR-0042): fresh connections carry no resume identity;
## clients following a Redirect resume with player_id/ship_id.
func test_hello_command_carries_no_resume_identity_when_ids_are_negative() -> void:
	var bytes: PackedByteArray = _cmd.hello_command(-1, -1)
	var d: Dictionary = _decoder.decode(bytes)
	assert_str(d["type"]).is_equal("Hello")
	assert_object(d["resume"]).is_null()


func test_hello_command_carries_a_resume_identity_when_ids_are_present() -> void:
	var bytes: PackedByteArray = _cmd.hello_command(7, 42)
	var d: Dictionary = _decoder.decode(bytes)
	var resume: Dictionary = d["resume"]
	assert_int(typeof(resume["player_id"])).is_equal(TYPE_INT)
	assert_int(typeof(resume["ship_id"])).is_equal(TYPE_INT)
	assert_int(int(resume["player_id"])).is_equal(7)
	assert_int(int(resume["ship_id"])).is_equal(42)
