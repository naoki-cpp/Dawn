## GDExtension contract tests for typed Sector and Market request builders.
extends GdUnitTestSuite


var _command: ClientCommand


func before_test() -> void:
	_command = ClientCommand.new()


func test_sector_builder_returns_typed_success_result() -> void:
	var result: ClientCommandResult = _command.stop_command()

	assert_bool(result.ok).is_true()
	assert_bool(result.bytes.is_empty()).is_false()
	assert_str(result.error_code).is_empty()
	assert_str(result.error_message).is_empty()


func test_sector_builder_reports_invalid_input_without_a_byte_sentinel() -> void:
	var result: ClientCommandResult = _command.lock_on_command(-1)

	assert_bool(result.ok).is_false()
	assert_bool(result.bytes.is_empty()).is_true()
	assert_str(result.error_code).is_equal("invalid_id")
	assert_bool(result.error_message.is_empty()).is_false()


func test_station_warp_builder_returns_typed_success_result() -> void:
	var result: ClientCommandResult = _command.warp_to_station_command(4)

	assert_bool(result.ok).is_true()
	assert_bool(result.bytes.is_empty()).is_false()


func test_market_builders_are_dedicated_typed_methods() -> void:
	assert_bool(_command.has_method("market_build")).is_false()

	var refresh: ClientCommandResult = _command.market_refresh_command()
	assert_bool(refresh.ok).is_true()

	var place: ClientCommandResult = _command.market_place_order_command(
		7, ItemIdentity.scrap_metal(), "Ask", 10, 2)
	assert_bool(place.ok).is_true()

	var cancel: ClientCommandResult = _command.market_cancel_order_command(3)
	assert_bool(cancel.ok).is_true()


func test_market_builders_validate_side_and_ids_explicitly() -> void:
	var invalid_side: ClientCommandResult = _command.market_place_order_command(
		7, ItemIdentity.scrap_metal(), "Offer", 10, 2)
	assert_bool(invalid_side.ok).is_false()
	assert_str(invalid_side.error_code).is_equal("invalid_market_side")

	var invalid_order: ClientCommandResult = _command.market_cancel_order_command(-1)
	assert_bool(invalid_order.ok).is_false()
	assert_str(invalid_order.error_code).is_equal("invalid_id")


func test_hello_builder_reports_invalid_resume_ticket() -> void:
	var invalid_ticket := PackedByteArray([1, 2, 3])
	var result: ClientCommandResult = _command.hello_command(invalid_ticket)

	assert_bool(result.ok).is_false()
	assert_str(result.error_code).is_equal("invalid_resume_ticket")
