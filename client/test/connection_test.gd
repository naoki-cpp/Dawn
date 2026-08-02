## connection_test.gd
##
## Signal and redirect wiring tests for connection.gd. Wire decoding and
## variant projection are covered in Rust; these tests exercise the typed
## outcome boundary with Rust-owned session/loadout state bound.
extends GdUnitTestSuite

const Connection = preload("res://scripts/connection.gd")
const Main = preload("res://scripts/main.gd")


func _state() -> Array:
	return [WorldSession.new(), PlayerLoadout.new()]


func test_reconnect_logging_is_emitted_on_first_attempt_and_after_interval() -> void:
	assert_bool(Connection.should_log_reconnect(1, 0.0, 30.0)).is_true()
	assert_bool(Connection.should_log_reconnect(10, 29.9, 30.0)).is_false()
	assert_bool(Connection.should_log_reconnect(10, 30.0, 30.0)).is_true()


func test_normalize_ws_url_adds_ws_scheme_to_host_port() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
	connection.free()


func test_normalize_ws_url_keeps_existing_ws_scheme() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("ws://127.0.0.1:7880")).is_equal("ws://127.0.0.1:7880")
	connection.free()


func test_normalize_ws_url_keeps_existing_wss_scheme() -> void:
	var connection: Node = Connection.new()
	assert_str(connection._normalize_ws_url("wss://example.test/ws")).is_equal("wss://example.test/ws")
	connection.free()


func test_welcome_outcome_updates_identity_and_emits_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.welcomed.connect(func(player_id: int, ship_id: int) -> void:
		received.append({"player_id": player_id, "ship_id": ship_id})
	)

	connection._accept_welcome(5, 11)

	assert_int(connection.player_id).is_equal(5)
	assert_int(connection.ship_id).is_equal(11)
	assert_bool(connection._welcomed).is_true()
	assert_int(received.size()).is_equal(1)
	connection.free()


func test_module_activated_outcome_emits_module_signal() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.module_activated.connect(func(ship_id: int, module_id: int, slot: String) -> void:
		received.append({"ship_id": ship_id, "module_id": module_id, "slot": slot})
	)

	connection._accept_module_activated(11, 7, "Mid")

	assert_int(received.size()).is_equal(1)
	assert_int((received[0] as Dictionary)["ship_id"]).is_equal(11)
	assert_int((received[0] as Dictionary)["module_id"]).is_equal(7)
	assert_str((received[0] as Dictionary)["slot"]).is_equal("Mid")
	connection.free()


func test_player_loadout_outcome_emits_after_typed_state_replacement() -> void:
	var connection: Node = Connection.new()
	var received: Array = []
	connection.player_fitting_received.connect(func() -> void:
		received.append(true)
	)

	connection._accept_player_loadout()

	assert_int(received.size()).is_equal(1)
	connection.free()


func test_real_outcome_dispatches_welcome_and_detects_missing_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("Welcome")
	assert_object(outcome).is_not_null()
	var state := _state()

	var missing_target := Node.new()
	assert_bool(outcome.dispatch(missing_target, state[0], state[1], -1)).is_false()
	missing_target.free()

	var connection: Node = Connection.new()
	assert_bool(outcome.dispatch(connection, state[0], state[1], -1)).is_true()
	assert_int(connection.player_id).is_equal(5)
	assert_int(connection.ship_id).is_equal(11)
	connection.free()


class EventDispatchTarget:
	extends RefCounted
	var left_ship_id: int = -1
	var removed: bool = true

	func _handle_aoi_leave(ship_id: int, was_removed: bool) -> void:
		left_ship_id = ship_id
		removed = was_removed


class MotionPathShip:
	extends Node3D
	var reconcile_calls: Array[Dictionary] = []

	func reconcile_motion(
		position: PackedFloat64Array, velocity: Vector3, tick: int
	) -> void:
		reconcile_calls.append({
			"position": position,
			"velocity": velocity,
			"tick": tick,
		})


func test_real_world_event_outcome_dispatches_to_typed_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var top_level: ServerMessageOutcome = decoder.test_outcome("AoiLeave")
	var connection: Node = Connection.new()
	var state := _state()
	var events: Array = []
	connection.event_received.connect(func(event: ServerEventOutcome) -> void:
		events.append(event)
	)
	assert_bool(top_level.dispatch(connection, state[0], state[1], -1)).is_true()
	assert_int(events.size()).is_equal(1)

	var target := EventDispatchTarget.new()
	assert_bool((events[0] as ServerEventOutcome).dispatch(target)).is_true()
	assert_int(target.left_ship_id).is_equal(19)
	assert_bool(target.removed).is_false()
	connection.free()


func test_initial_state_updates_session_even_when_presentation_handler_is_missing() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var target := RefCounted.new()
	var state := _state()

	assert_bool(outcome.dispatch(target, state[0], state[1], 11)).is_false()
	assert_str((state[0] as WorldSession).current_system_name()).is_equal("Alpha")
	assert_int((state[0] as WorldSession).player_ship_id()).is_equal(11)


func test_real_initial_state_updates_session_before_typed_signal() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var connection: Node = Connection.new()
	var state := _state()
	var states: Array = []
	connection.initial_state_received.connect(func(initial: InitialStatePresentation) -> void:
		states.append(initial)
	)
	assert_bool(outcome.dispatch(connection, state[0], state[1], 11)).is_true()
	assert_int(states.size()).is_equal(1)
	assert_int((states[0] as InitialStatePresentation).ships.size()).is_equal(2)
	assert_str((state[0] as WorldSession).current_system_name()).is_equal("Alpha")
	assert_int((state[0] as WorldSession).player_ship_id()).is_equal(11)
	connection.free()


func test_real_market_outcome_preserves_every_item_variant() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("MarketSnapshot")
	var connection: Node = Connection.new()
	var state := _state()
	var snapshots: Array = []
	connection.market_snapshot_received.connect(func(snapshot: MarketSnapshot) -> void:
		snapshots.append(snapshot)
	)
	assert_bool(outcome.dispatch(connection, state[0], state[1], -1)).is_true()
	var snapshot := snapshots[0] as MarketSnapshot
	assert_int(snapshot.balance).is_equal(250)
	assert_str(snapshot.notice).is_equal("Ready")
	assert_int(snapshot.orders.size()).is_equal(3)
	var scrap: ItemIdentity = (snapshot.orders[0] as MarketOrder).item_id
	var module: ItemIdentity = (snapshot.orders[1] as MarketOrder).item_id
	var ship: ItemIdentity = (snapshot.orders[2] as MarketOrder).item_id
	assert_bool(scrap.is_scrap_metal()).is_true()
	assert_bool(module.is_module()).is_true()
	assert_int(module.module_id() as int).is_equal(3)
	assert_bool(ship.is_packaged_ship()).is_true()
	assert_int(ship.ship_type_id() as int).is_equal(7)
	connection.free()


func test_real_motion_correction_emits_typed_presentation() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("MotionCorrection")
	var connection: Node = Connection.new()
	var state := _state()
	var corrections: Array = []
	connection.motion_correction_received.connect(
		func(correction: MotionCorrectionPresentation) -> void:
			corrections.append(correction)
	)

	assert_bool(outcome.dispatch(connection, state[0], state[1], 11)).is_true()
	assert_int(corrections.size()).is_equal(1)
	var correction := corrections[0] as MotionCorrectionPresentation
	assert_int(correction.ship_id).is_equal(11)
	assert_int(correction.tick).is_equal(42)
	assert_array(correction.position).contains_exactly([
		5.0 * 1.495978707e11 + 10.0, 20.0, 30.0,
	])
	connection.free()


func test_real_motion_correction_reaches_main_scene_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("MotionCorrection")
	var connection: Node = Connection.new()
	var main: Node = Main.new()
	var ship := MotionPathShip.new()
	main._ships = {11: ship}
	main._player_ship_id = 11
	main.add_child(ship)
	connection.motion_correction_received.connect(
		Callable(main, "_handle_motion_correction"))

	assert_bool(outcome.dispatch(connection, main._session, main._loadout, 11)).is_true()
	assert_int(ship.reconcile_calls.size()).is_equal(1)
	var call: Dictionary = ship.reconcile_calls[0]
	assert_array(call["position"]).contains_exactly([
		5.0 * 1.495978707e11 + 10.0, 20.0, 30.0,
	])
	assert_vector(call["velocity"]).is_equal(Vector3(4.0, 5.0, -6.0))
	assert_int(call["tick"]).is_equal(42)

	main.free()
	connection.free()


class DockEventTarget:
	extends RefCounted
	var accepted: bool = false

	func _handle_ship_docked(
		_ship_id: int, _station_id: int, _tick: int, session_accepted: bool
	) -> void:
		accepted = session_accepted


func test_real_dock_event_updates_session_before_typed_event_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var initial: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var docked: ServerMessageOutcome = decoder.test_outcome("ShipDocked")
	var connection: Node = Connection.new()
	var state := _state()
	var events: Array = []
	connection.event_received.connect(func(event: ServerEventOutcome) -> void:
		events.append(event)
	)

	assert_bool(initial.dispatch(connection, state[0], state[1], 11)).is_true()
	assert_bool(docked.dispatch(connection, state[0], state[1], 11)).is_true()
	assert_bool((state[0] as WorldSession).is_docked()).is_true()
	assert_int((state[0] as WorldSession).docked_station_id()).is_equal(5)
	assert_str((state[0] as WorldSession).docked_station_name()).is_equal("Forge Station")

	var target := DockEventTarget.new()
	assert_bool((events.back() as ServerEventOutcome).dispatch(target)).is_true()
	assert_bool(target.accepted).is_true()
	connection.free()
