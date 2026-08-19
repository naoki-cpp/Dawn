## GDExtension boundary tests for the typed station inventory policy.
extends GdUnitTestSuite


var _policy: StationInventoryInteraction


func before_test() -> void:
	_policy = StationInventoryInteraction.new()


func test_shipless_docked_player_can_assemble_and_select_a_ship() -> void:
	var assemble: StationInventoryAction = _policy.click(
		StationInventoryRow.station(ItemIdentity.packaged_ship(7) as ItemIdentity),
		-1, 3, [])
	assert_bool(assemble.request_count() == 1).is_true()
	assert_bool(assemble.request_result().ok).is_true()

	var select: StationInventoryAction = _policy.click(
		StationInventoryRow.owned_ship(9, false) as StationInventoryRow,
		-1, -1, [])
	assert_bool(select.request_count() == 1).is_true()
	assert_bool(select.request_result().ok).is_true()


func test_build_and_disassemble_require_active_docked_context() -> void:
	var build: StationInventoryRow = StationInventoryRow.build_ship_type(7) as StationInventoryRow
	var disassemble: StationInventoryRow = StationInventoryRow.disassemble()
	assert_int(_policy.click(build, -1, 3, []).request_count()).is_equal(0)
	assert_int(_policy.click(disassemble, 1, -1, []).request_count()).is_equal(0)
	assert_int(_policy.click(build, 1, 3, []).request_count()).is_equal(1)
	assert_int(_policy.click(disassemble, 1, 3, []).request_count()).is_equal(1)


func test_build_picker_is_local_and_cargo_directions_are_typed() -> void:
	var toggle: StationInventoryAction = _policy.click(
		StationInventoryRow.build_toggle(), -1, -1, [])
	assert_bool(toggle.is_build_picker_toggle()).is_true()
	assert_int(toggle.request_count()).is_equal(0)

	var cargo: StationInventoryRow = StationInventoryRow.cargo(
		ItemIdentity.scrap_metal(), "")
	var station: StationInventoryRow = StationInventoryRow.station(
		ItemIdentity.scrap_metal())
	var to_station: StationInventoryAction = _policy.resolve_drop(cargo, 2, station, 1, 3)
	var to_ship: StationInventoryAction = _policy.resolve_drop(station, 1, cargo, 1, 3)
	assert_int(to_station.request_count()).is_equal(1)
	assert_int(to_ship.request_count()).is_equal(1)
	assert_bool(to_station.request_result().ok).is_true()
	assert_bool(to_ship.request_result().ok).is_true()


func test_same_column_and_invalid_rows_are_no_ops() -> void:
	var cargo: StationInventoryRow = StationInventoryRow.cargo(
		ItemIdentity.scrap_metal(), "")
	var no_op: StationInventoryAction = _policy.resolve_drop(cargo, 1, cargo, 1, 3)
	assert_int(no_op.request_count()).is_equal(0)
	assert_bool(no_op.is_build_picker_toggle()).is_false()
	assert_bool(StationInventoryRow.fitted(0, "High") == null).is_true()
