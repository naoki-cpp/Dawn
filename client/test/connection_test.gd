## Inbound connection and presentation-seam contract tests.
##
## The Rust outcome commits typed client state and calls the final world target
## once. connection.gd owns only connection lifecycle and transport callbacks.
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


func test_welcome_outcome_updates_identity_resume_ticket_and_welcome_state() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("Welcome")
	var state := _state()
	var connection: Node = Connection.new()

	assert_bool(outcome.dispatch(connection, connection, state[0], state[1], -1)).is_true()
	assert_int(connection.player_id).is_equal(5)
	assert_int(connection.ship_id).is_equal(11)
	assert_int(connection._resume_ticket.size()).is_equal(32)
	assert_bool(connection._welcomed).is_true()
	connection.free()


class DirectPresentationTarget:
	extends RefCounted
	var session: WorldSession
	var loadout: PlayerLoadout
	var initial_state_seen: bool = false
	var initial_system: String = ""
	var initial_ship_count: int = 0
	var fitting_tick: int = -1
	var module_active: bool = false
	var module_reason: String = ""
	var market_snapshot: MarketSnapshot
	var motion_correction: MotionCorrectionPresentation

	func _on_initial_state(state: InitialStatePresentation) -> void:
		initial_state_seen = true
		initial_system = session.current_system_name()
		initial_ship_count = state.ships.size()

	func _on_player_fitting() -> void:
		fitting_tick = loadout.tick()

	func _on_module_activated(_ship_id: int, _module_id: int, _slot: String) -> void:
		module_active = (loadout.modules()[0] as ModuleRow).is_active

	func _on_module_deactivated(
		_ship_id: int, _module_id: int, _slot: String, reason: String
	) -> void:
		module_active = (loadout.modules()[0] as ModuleRow).is_active
		module_reason = reason

	func _on_market_snapshot(snapshot: MarketSnapshot) -> void:
		market_snapshot = snapshot

	func _handle_motion_correction(correction: MotionCorrectionPresentation) -> void:
		motion_correction = correction


func _direct_target(state: Array) -> DirectPresentationTarget:
	var target := DirectPresentationTarget.new()
	target.session = state[0]
	target.loadout = state[1]
	return target


func _dispatch(kind: String, target: DirectPresentationTarget, state: Array) -> bool:
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
	assert_object(outcome).is_not_null()
	return outcome.dispatch(RefCounted.new(), target, state[0], state[1], 11)


func test_initial_state_commits_before_calling_the_final_world_handler() -> void:
	var state := _state()
	var target := _direct_target(state)

	assert_bool(_dispatch("InitialState", target, state)).is_true()
	assert_bool(target.initial_state_seen).is_true()
	assert_int(target.initial_ship_count).is_equal(2)
	assert_str(target.initial_system).is_equal("Alpha")
	assert_str((state[0] as WorldSession).current_system_name()).is_equal("Alpha")
	assert_int((state[0] as WorldSession).player_ship_id()).is_equal(11)


func test_player_loadout_commits_before_calling_the_final_world_handler() -> void:
	var state := _state()
	var target := _direct_target(state)

	assert_bool(_dispatch("PlayerLoadoutSwitch", target, state)).is_true()
	assert_int(target.fitting_tick).is_equal(12)
	assert_int((state[1] as PlayerLoadout).active_ship_id()).is_equal(22)


func test_module_activation_commits_before_calling_the_final_world_handler() -> void:
	var state := _state()
	var module := ModuleRow.test_fixture("Mid", 0, 7, "", "", false, true, 0.0, 10)
	assert_bool((state[1] as PlayerLoadout).test_fixture(
		0, [module], -1, "", 11, []
	)).is_true()
	var target := _direct_target(state)

	assert_bool(_dispatch("ModuleActivated", target, state)).is_true()
	assert_bool(target.module_active).is_true()


func test_module_deactivation_commits_before_calling_the_final_world_handler() -> void:
	var state := _state()
	var module := ModuleRow.test_fixture("Mid", 0, 7, "", "", true, true, 0.0, 10)
	assert_bool((state[1] as PlayerLoadout).test_fixture(
		0, [module], -1, "", 11, []
	)).is_true()
	var target := _direct_target(state)

	assert_bool(_dispatch("ModuleDeactivated", target, state)).is_true()
	assert_bool(target.module_active).is_false()
	assert_str(target.module_reason).is_equal("cap")


func test_market_snapshot_reaches_the_final_world_handler_without_a_connection_relay() -> void:
	var state := _state()
	var target := _direct_target(state)

	assert_bool(_dispatch("MarketSnapshot", target, state)).is_true()
	assert_object(target.market_snapshot).is_not_null()
	assert_int(target.market_snapshot.balance).is_equal(250)
	assert_str(target.market_snapshot.notice).is_equal("Ready")
	assert_int(target.market_snapshot.orders.size()).is_equal(3)
	var scrap: ItemIdentity = (target.market_snapshot.orders[0] as MarketOrder).item_id
	var module: ItemIdentity = (target.market_snapshot.orders[1] as MarketOrder).item_id
	var ship: ItemIdentity = (target.market_snapshot.orders[2] as MarketOrder).item_id
	assert_bool(scrap.is_scrap_metal()).is_true()
	assert_bool(module.is_module()).is_true()
	assert_int(module.module_id() as int).is_equal(3)
	assert_bool(ship.is_packaged_ship()).is_true()
	assert_int(ship.ship_type_id() as int).is_equal(7)


func test_motion_correction_reaches_the_final_world_handler_without_a_connection_relay() -> void:
	var state := _state()
	var target := _direct_target(state)

	assert_bool(_dispatch("MotionCorrection", target, state)).is_true()
	assert_object(target.motion_correction).is_not_null()
	assert_int(target.motion_correction.ship_id).is_equal(11)
	assert_int(target.motion_correction.tick).is_equal(42)


class EventDispatchTarget:
	extends RefCounted
	var left_ship_id: int = -1
	var removed: bool = true

	func _handle_aoi_leave(ship_id: int, was_removed: bool) -> void:
		left_ship_id = ship_id
		removed = was_removed


func test_existing_world_fact_still_dispatches_directly_to_its_final_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("AoiLeave")
	var state := _state()
	var target := EventDispatchTarget.new()

	assert_bool(outcome.dispatch(RefCounted.new(), target, state[0], state[1], -1)).is_true()
	assert_int(target.left_ship_id).is_equal(19)
	assert_bool(target.removed).is_false()


func test_state_commits_even_when_the_final_handler_is_missing() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var target := RefCounted.new()
	var state := _state()

	assert_bool(outcome.dispatch(target, target, state[0], state[1], 11)).is_false()
	assert_str((state[0] as WorldSession).current_system_name()).is_equal("Alpha")
	assert_int((state[0] as WorldSession).player_ship_id()).is_equal(11)


func test_world_fact_commits_even_when_the_final_handler_is_missing() -> void:
	var decoder := ServerMessageDecoder.new()
	var initial: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var docked: ServerMessageOutcome = decoder.test_outcome("ShipDocked")
	var target := RefCounted.new()
	var state := _state()

	assert_bool(initial.dispatch(target, target, state[0], state[1], 11)).is_false()
	assert_bool(docked.dispatch(target, target, state[0], state[1], 11)).is_false()
	assert_bool((state[0] as WorldSession).is_docked()).is_true()
	assert_int((state[0] as WorldSession).docked_station_id()).is_equal(5)


class DockEventTarget:
	extends RefCounted
	var session: WorldSession
	var accepted: bool = false
	var station_name_at_callback: String = ""

	func _on_initial_state(_state: InitialStatePresentation) -> void:
		pass

	func _handle_ship_docked(
		_ship_id: int, _station_id: int, _tick: int, session_accepted: bool
	) -> void:
		accepted = session_accepted
		station_name_at_callback = session.docked_station_name()


func test_ship_docked_commits_station_state_before_the_final_handler() -> void:
	var decoder := ServerMessageDecoder.new()
	var initial: ServerMessageOutcome = decoder.test_outcome("InitialState")
	var docked: ServerMessageOutcome = decoder.test_outcome("ShipDocked")
	var state := _state()
	var target := DockEventTarget.new()
	target.session = state[0]

	assert_bool(initial.dispatch(RefCounted.new(), target, state[0], state[1], 11)).is_true()
	assert_bool(docked.dispatch(RefCounted.new(), target, state[0], state[1], 11)).is_true()
	assert_bool(target.accepted).is_true()
	assert_str(target.station_name_at_callback).is_equal("Forge Station")
	assert_int((state[0] as WorldSession).docked_station_id()).is_equal(5)


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


func test_motion_correction_calls_main_handler_directly() -> void:
	var decoder := ServerMessageDecoder.new()
	var outcome: ServerMessageOutcome = decoder.test_outcome("MotionCorrection")
	var main: Node = Main.new()
	var ship := MotionPathShip.new()
	main._ships = {11: ship}
	main._player_ship_id = 11
	main.add_child(ship)

	assert_bool(outcome.dispatch(RefCounted.new(), main, main._session, main._loadout, 11)).is_true()
	assert_int(ship.reconcile_calls.size()).is_equal(1)
	var call: Dictionary = ship.reconcile_calls[0]
	assert_array(call["position"]).contains_exactly([
		5.0 * 1.495978707e11 + 10.0, 20.0, 30.0,
	])
	assert_vector(call["velocity"]).is_equal(Vector3(4.0, 5.0, -6.0))
	assert_int(call["tick"]).is_equal(42)
	main.free()
