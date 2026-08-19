## main_test.gd
##
## Unit tests for the pure-math helpers still in main.gd: coordinate
## conversion and module-deactivation bookkeeping. These are the only parts of main.gd
## testable without a running scene tree -- HUD construction and input
## routing depend on @onready scene paths and need the Godot editor to
## verify (see docs/architecture/architecture-review/client.md C-1). Picking math lives
## in ship_picking.gd (client/test/ship_picking_test.gd); marker spawning
## and spectral colour live in navigation_marker_renderer.gd
## (client/test/navigation_marker_renderer_test.gd).
extends GdUnitTestSuite

const __source: String = "res://scripts/main.gd"
const MainScript = preload("res://scripts/main.gd")
const InventoryRow = preload("res://scripts/inventory_row.gd")
const HudManager = preload("res://scripts/hud_manager.gd")
const AU_M: float = 1.495978707e11

var _main: Node


class TypedOutcomeTarget:
	extends RefCounted

	func _accept_initial_state(_state: InitialStatePresentation) -> void:
		pass

	func _accept_player_loadout() -> void:
		pass

	func _handle_ship_docked(
		_ship_id: int, _station_id: int, _tick: int, _session_accepted: bool
	) -> void:
		pass


func _dispatch_fixture(kind: String, connection_ship_id: int = -1) -> void:
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
	assert_object(outcome).is_not_null()
	var target := TypedOutcomeTarget.new()
	assert_bool(outcome.dispatch(
		target, target, _main._session, _main._loadout, connection_ship_id
	)).is_true()


func _dispatch_to_main(kind: String, connection_ship_id: int = -1) -> void:
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome(kind)
	assert_object(outcome).is_not_null()
	var connection_target := TypedOutcomeTarget.new()
	assert_bool(outcome.dispatch(
		connection_target, _main, _main._session, _main._loadout, connection_ship_id
	)).is_true()


func _setup_docked_session() -> void:
	## Server-derived session state must use the same typed inbound path as
	## production. The fixture's player ship is 11 and station is 5; tests may
	## still use a different optimistic `_player_ship_id` for command arguments.
	_dispatch_fixture("InitialState", 11)
	_dispatch_fixture("ShipDocked", 11)


class FakeShip:
	extends Node3D

	var velocity_calls: Array[Vector3] = []
	var velocity_tick_calls: Array[int] = []
	var thrust_calls: Array[Vector3] = []
	var set_as_player_calls: int = 0
	var clear_as_player_calls: int = 0
	var reconcile_calls: Array[Dictionary] = []
	var dock_calls: Array[Dictionary] = []
	var undock_calls: Array[Dictionary] = []
	var server_position_value := PackedFloat64Array([0.0, 0.0, 0.0])

	func configure_motion(
		_max_speed: float,
		_mass: float,
		_inertia_modifier: float,
		p: PackedFloat64Array,
		v: Vector3,
		_tick: int = 0
	) -> void:
		server_position_value = p
		velocity_calls.append(v)

	func set_velocity(v: Vector3, tick: int = 0) -> bool:
		velocity_calls.append(v)
		velocity_tick_calls.append(tick)
		return true

	## WorldPresentation.attach_player_ship() calls this via ship.call(...);
	## a real ship (ship_controller.gd) sets up player-only visuals here.
	func set_as_player() -> void:
		set_as_player_calls += 1

	## WorldPresentation.attach_player_ship() calls this on the previously
	## piloted ship (if any) via ship.call(...) when a different ship
	## becomes the active one; a real ship tears down player-only visuals.
	func clear_as_player() -> void:
		clear_as_player_calls += 1

	func set_thrust_direction(v: Vector3) -> void:
		thrust_calls.append(v)

	func set_braking() -> void:
		thrust_calls.append(Vector3.ZERO)

	func reset_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> void:
		server_position_value = p
		velocity_calls.append(v)
		thrust_calls.append(Vector3.ZERO)

	func dock_motion(p: PackedFloat64Array, _tick: int) -> bool:
		server_position_value = p
		velocity_calls.append(Vector3.ZERO)
		thrust_calls.append(Vector3.ZERO)
		dock_calls.append({"position": p, "tick": _tick})
		return true

	func undock_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> bool:
		server_position_value = p
		velocity_calls.append(v)
		thrust_calls.append(Vector3.ZERO)
		undock_calls.append({"position": p, "velocity": v, "tick": _tick})
		return true

	func reconcile_motion(p: PackedFloat64Array, v: Vector3, _tick: int) -> void:
		server_position_value = p
		velocity_calls.append(v)
		reconcile_calls.append({"position": p, "velocity": v, "tick": _tick})

	func server_position() -> PackedFloat64Array:
		return server_position_value

	func get_speed_server() -> float:
		return 0.0


class FakeWorldPresentation:
	extends WorldPresentation

	func attach_player_ship(ship: Node3D, _weapon_range: float, _weapon_falloff: float) -> void:
		if ship == null:
			return
		if _player_ship != ship:
			detach_player_ship()
		_player_ship = ship
		ship.call("set_as_player")

	func detach_player_ship() -> void:
		if _player_ship != null and is_instance_valid(_player_ship):
			_player_ship.call("clear_as_player")
		_player_ship = null

	func update_tactical_overlay_ranges(_weapon_range: float, _weapon_falloff: float) -> void:
		pass


class TestableMain:
	extends MainScript

	var instantiated_ships: Array[Node3D] = []

	func _instantiate_ship(sid: int, server_pos: PackedFloat64Array) -> Node3D:
		var ship := FakeShip.new()
		ship.name = "Ship_%d" % sid
		ship.server_position_value = server_pos
		add_child(ship)
		instantiated_ships.append(ship)
		return ship


class FakeConnection:
	extends Node

	func is_connected_to_server() -> bool:
		return false

	var station_inventory_actions: Array[StationInventoryAction] = []

	func send_station_inventory_action(action: StationInventoryAction) -> void:
		station_inventory_actions.append(action)

	var activate_calls: Array[Dictionary] = []
	var deactivate_calls: Array[Dictionary] = []

	func send_activate_module(module_id: int, slot: String, target_ship_id: int = -1) -> void:
		activate_calls.append({
			"module_id": module_id,
			"slot": slot,
			"target_ship_id": target_ship_id,
		})

	func send_deactivate_module(module_id: int, slot: String) -> void:
		deactivate_calls.append({"module_id": module_id, "slot": slot})

func before_test() -> void:
	## .new() without adding to the scene tree never triggers _ready(), so
	## the @onready scene-path vars stay null -- fine, since none of the
	## functions under test touch them.
	_main = load(__source).new()
	## _ready() normally injects WorldSpace through WorldPresentation.build().
	## This fixture skips _ready(), so establish the same production dependency.
	_initialize_main_dependencies()


func _initialize_main_dependencies() -> void:
	_main._presentation._world = _main._world
	_main._interaction = load("res://scripts/world_interaction.gd").new()
	_main._loadout = PlayerLoadout.new()


func _replace_with_testable_main() -> void:
	_main.free()
	## The replacement root is intentionally outside the scene tree so its
	## _ready() hook does not run. Register it with GdUnit4 nevertheless, so
	## its child test ships are released even if a test exits before cleanup.
	_main = auto_free(TestableMain.new())
	_main._presentation = FakeWorldPresentation.new()
	_initialize_main_dependencies()


func after_test() -> void:
	if is_instance_valid(_main):
		_main.free()
	_main = null


func _module_fixture(
	module_id: int,
	slot: String,
	active: bool,
	kind: String = "",
	index: int = 0
) -> ModuleRow:
	return ModuleRow.test_fixture(
		slot, index, module_id, "Test Module", kind, active, true, 0.0, 10
	)


func _set_loadout_modules(modules: Array[ModuleRow]) -> void:
	var owned_ships: Array[OwnedShipRow] = []
	assert_bool(_main._loadout.test_fixture(
		0, modules, -1, "", -1, owned_ships
	)).is_true()


func _setup_pending_docked_switch() -> FakeShip:
	_replace_with_testable_main()
	var old_ship := FakeShip.new()
	_main.add_child(old_ship)
	_main._ships = {11: old_ship}
	_dispatch_fixture("InitialState", 11)
	_main._set_as_player_ship(11, old_ship)
	_dispatch_fixture("ShipDocked", 11)
	_dispatch_fixture("PlayerLoadoutUnknownDocked", 11)
	_main._apply_current_dock_state_to_player_ship(old_ship)
	assert_int(_main._session.player_ship_id()).is_equal(11)
	assert_bool(_main._session.is_docked()).is_true()
	assert_int(old_ship.dock_calls.size()).is_equal(1)
	return old_ship


func test_warp_hud_guidance_uses_shared_minimum_distance_boundary() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	var stats_label: Label = auto_free(Label.new())
	_main._hud_surface.build(auto_free(Node.new()), hud, stats_label)

	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1
	var min_warp_distance: float = _main._client_rules.min_warp_distance()
	var gate: GateRecord = _gate(7, PackedFloat64Array([
		min_warp_distance - 1.0, 0.0, 0.0]), 2000.0, "Beta")
	_main._gates = [gate]
	_main._interaction.interpret_primary_click(
		Vector2.ZERO, 1.0, false, _main._player_ship_id, -1, gate.gate_id, -1)

	_main._update_hud()
	assert_bool(stats_label.text.contains("[W] too close to warp")).is_true()
	assert_bool(stats_label.text.contains("[J] Warp+Jump")).is_false()

	gate.position = PackedFloat64Array([min_warp_distance, 0.0, 0.0])
	_main._update_hud()
	assert_bool(stats_label.text.contains("[W] too close to warp")).is_false()
	assert_bool(stats_label.text.contains("[W] Warp  [J] Warp+Jump")).is_true()

	ship.free()
	connection.free()


# -- _handle_position_snap ----------------------------------------------------

func test_player_position_snap_clears_residual_warp_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.global_position = Vector3(10.0, 0.0, 0.0)
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	_main._handle_position_snap(
		1, PackedFloat64Array([1_000_000.0, 0.0, 0.0]))

	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_observed_ship_position_snap_clears_residual_warp_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {2: ship}
	_main._player_ship_id = 1

	_main._handle_position_snap(
		2, PackedFloat64Array([100.0, 20.0, 300.0]))

	assert_array(ship.server_position_value).contains_exactly([100.0, 20.0, 300.0])
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_ship_docked_event_clears_residual_motion() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.global_position = Vector3(99.0, 99.0, 99.0)
	_main._ships = {2: ship}
	_main._stations = [_station(0, "Forge Station",
		PackedFloat64Array([5.0 * AU_M + 10.0, 20.0, 300.0]), 0.0)]
	_main._world.rebase_to_components(5.0 * AU_M, 0.0, 0.0)

	_main._handle_ship_docked(2, 0, 12, true)

	## FakeShip is not attached to a live SceneTree here, so `position` is the
	## stable seam for verifying the dock snap.
	assert_array(ship.server_position_value).contains_exactly([
		5.0 * AU_M + 10.0, 20.0, 300.0,
	])
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	assert_vector(ship.thrust_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_ship_docked_event_stops_ship_when_station_map_is_not_ready() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	ship.server_position_value = PackedFloat64Array([9.0, 8.0, 7.0])
	_main._ships = {2: ship}

	_main._handle_ship_docked(2, 99, 12, true)

	assert_int(ship.dock_calls.size()).is_equal(1)
	assert_array(ship.dock_calls[0]["position"]).contains_exactly([9.0, 8.0, 7.0])
	assert_vector(ship.velocity_calls.back()).is_equal(Vector3.ZERO)
	ship.free()


func test_rejected_player_undock_event_does_not_move_the_ship() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {2: ship}
	_main._player_ship_id = 2

	## Ordering acceptance is a WorldSessionState Rust test. This test covers
	## only the presentation contract for a rejected typed outcome.
	_main._handle_ship_undocked(2, 0, 11, false)

	assert_int(ship.undock_calls.size()).is_equal(0)
	ship.free()


func test_player_loadout_refresh_preserves_docked_motion_state() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {11: ship}
	_main._player_ship_id = 11
	_setup_docked_session()
	var modules: Array[ModuleRow] = []
	var owned_ships: Array[OwnedShipRow] = []
	assert_bool(_main._loadout.test_fixture(
		12, modules, 5, "Forge Station", 11, owned_ships
	)).is_true()

	_main._apply_loadout_side_effects()

	assert_int(ship.dock_calls.size()).is_equal(1)
	assert_int(ship.undock_calls.size()).is_equal(0)
	ship.free()


func test_au_navigation_proximity_uses_unquantized_positions() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1
	_main._world.rebase_to_components(5.0 * AU_M, 0.0, 0.0)
	_main._gates = [_gate(7,
		PackedFloat64Array([5.0 * AU_M + 10.0, 0.0, 0.0]), 20.0, "")]
	_main._stations = [_station(5, "",
		PackedFloat64Array([5.0 * AU_M + 15.0, 0.0, 0.0]), 20.0)]

	_main._update_gate_proximity()
	_main._update_station_proximity()

	assert_int(_main._nearby_gate_id).is_equal(7)
	assert_array(_main._nearby_station_ids).contains_exactly([5])
	ship.free()


func test_motion_correction_reconciles_the_active_ship() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1

	var correction := MotionCorrectionPresentation.new()
	correction.ship_id = 1
	correction.position = PackedFloat64Array([100.0, 20.0, 300.0])
	correction.velocity = PackedFloat64Array([4.0, 5.0, -6.0])
	correction.tick = 42
	_main._handle_motion_correction(correction)

	assert_int(ship.reconcile_calls.size()).is_equal(1)
	var motion_call: Dictionary = ship.reconcile_calls.back()
	assert_array(motion_call["position"]).contains_exactly([100.0, 20.0, 300.0])
	assert_vector(motion_call["velocity"]).is_equal(Vector3(4.0, 5.0, -6.0))
	assert_int(motion_call["tick"]).is_equal(42)
	ship.free()


func test_velocity_changed_passes_the_authority_tick_to_the_ship() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}

	_main._handle_velocity_changed(
		1, PackedFloat64Array([4.0, 5.0, -6.0]), 42)

	assert_array(ship.velocity_tick_calls).contains_exactly([42])
	ship.free()


func test_motion_correction_preserves_small_motion_near_a_true_au_origin() -> void:
	var ship := FakeShip.new()
	_main.add_child(ship)
	_main._ships = {1: ship}
	_main._player_ship_id = 1
	_main._world.rebase_to_components(5.0 * 1.495978707e11, 0.0, 0.0)

	var correction := MotionCorrectionPresentation.new()
	correction.ship_id = 1
	correction.position = PackedFloat64Array([5.0 * AU_M + 10.0, 0.0, 0.0])
	correction.velocity = PackedFloat64Array([4.0, 0.0, 0.0])
	correction.tick = 43
	_main._handle_motion_correction(correction)

	var motion_call: Dictionary = ship.reconcile_calls.back()
	assert_array(motion_call["position"]).contains_exactly([
		5.0 * AU_M + 10.0, 0.0, 0.0,
	])
	ship.free()


func test_accepted_player_undock_event_updates_presentation_state() -> void:
	_main._player_ship_id = 2
	_main._nearby_station_ids = [] as Array[int]

	## Session acceptance/order is covered in Rust; the GDScript handler only
	## consumes the typed outcome's accepted bit.
	_main._handle_ship_undocked(2, 0, 13, true)

	assert_int(_main._nearby_station_ids[0]).is_equal(0)


# -- _on_module_deactivated (manual OFF vs system-forced OFF) -----------------------
#
# ModuleDeactivated now carries a server-authoritative reason ("cap" | "range"
# | "", ADR-0035), so the client trusts it directly instead of guessing from
# its own DeactivateModuleCommand sends (regression this replaced: every
# forced-off used to render as CAP! even when the real cause was out-of-range).

func test_module_activated_marks_matching_player_module_active() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "Mid", false)])

	## ServerMessageOutcome commits authoritative loadout state before the
	## presentation callback recalculates derived ranges.
	_main._loadout.apply_module_activation(5, true, "")
	_main._on_module_activated(1, 5, "Mid")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_true()
	assert_str(mod_dict.forced_reason).is_equal("")


func test_module_toggle_marks_module_active_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "Mid", false)])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_true()
	assert_str(mod_dict.forced_reason).is_equal("")
	assert_int(connection.activate_calls.size()).is_equal(1)
	connection.free()


func test_module_toggle_of_a_targeted_kind_without_a_locked_target_is_refused_client_side() -> void:
	## Sending Activate for a Weapon/Tackle/Remote-repair kind without a
	## Locked target is rejected server-side outright (ADR-0035), which
	## previously showed as an instant on-then-off flicker (optimistic
	## toggle immediately corrected by the PlayerFitting resync). The
	## client now refuses client-side instead, regardless of range.
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", false, "Weapon")])
	## Fresh _main has no player lock target.

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_of_a_targeted_kind_against_a_locked_but_out_of_aoi_target_is_refused_client_side() -> void:
	## Lock survives AoI leave (ADR-0019, WorldSession state
	## remove_ship(clear_lock=false)): the locked target can be gone from
	## _ships while player_lock_target still points at it (e.g. right after
	## the target warps away). With no node to read a position from, the
	## range guard used to be skipped entirely and the activation fell
	## through to the server -- which rejects it, producing the same
	## on-then-off flicker the visible-target range guard exists to
	## prevent. The client must refuse here too.
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	## This is explicitly optimistic GDScript-owned state (ADR-0046), unlike
	## authoritative WorldSession state.
	_main._player_lock_target = 99
	_main._ships = {} # target 99 is not in AoI; player ship 1 isn't either.
	_set_loadout_modules([_module_fixture(5, "High", false, "Weapon")])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_int(connection.activate_calls.size()).is_equal(0)
	connection.free()


func test_module_toggle_marks_module_inactive_before_server_echo() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._toggle_module_by_index(0)

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_str(mod_dict.forced_reason).is_equal("")
	assert_int(connection.deactivate_calls.size()).is_equal(1)
	connection.free()


func test_module_deactivated_with_no_reason_is_a_plain_off() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	## ServerMessageOutcome commits authoritative loadout state before the
	## presentation callback recalculates derived ranges.
	_main._loadout.apply_module_activation(5, false, "")
	_main._on_module_deactivated(1, 5, "High", "")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_bool(mod_dict.is_active).is_false()
	assert_str(mod_dict.forced_reason).is_equal("")


func test_module_deactivated_with_cap_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._loadout.apply_module_activation(5, false, "cap")
	_main._on_module_deactivated(1, 5, "High", "cap")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_str(mod_dict.forced_reason).is_equal("cap")


func test_module_deactivated_with_range_reason_flags_forced_reason() -> void:
	_main._player_ship_id = 1
	_set_loadout_modules([_module_fixture(5, "High", true)])

	_main._loadout.apply_module_activation(5, false, "range")
	_main._on_module_deactivated(1, 5, "High", "range")

	var mod_dict: ModuleRow = _main._loadout.modules()[0]
	assert_str(mod_dict.forced_reason).is_equal("range")


## Regression: switching active ship via the SHIPS roster (SelectActiveShip,
## ADR-0037) used to only update _player_ship_id and session state --
## bookkeeping the camera never reads. WorldPresentation.attach_player_ship()
## (which retargets the camera, applies the player material, and re-attaches
## the tactical overlay) was never called for this path, so the camera
## silently kept following the old ship.
func test_switching_active_ship_to_a_known_ship_reattaches_the_camera() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)
	var ship_b: FakeShip = auto_free(FakeShip.new())
	add_child(ship_b)
	ship_b.global_position = Vector3(100.0, 0.0, 0.0)

	_main._ships = {11: ship_a, 22: ship_b}
	_dispatch_fixture("InitialState", 11)
	## Route through the real attach path so the presentation owns the previous
	## player ship and can revert its visuals during the switch.
	_main._set_as_player_ship(11, ship_a)

	_dispatch_fixture("PlayerLoadoutSwitch", 11)
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(22)
	assert_int(_main._session.player_ship_id()).is_equal(22)
	assert_str(_main._session.player_ship_type_name()).is_equal("Venture")
	var health: ShipHealth = _main._session.player_health()
	assert_float(health.shield).is_equal_approx(210.0, 0.001)
	assert_float(health.max_shield).is_equal_approx(250.0, 0.001)
	var cap: CapacitorStatus = _main._session.capacitor_status()
	assert_float(cap.current).is_equal_approx(80.0, 0.001)
	assert_float(cap.max).is_equal_approx(80.0, 0.001)
	assert_float(cap.recharge).is_equal_approx(4.0, 0.001)
	assert_int(ship_b.set_as_player_calls).is_equal(1)
	assert_object(camera._target_node).is_equal(ship_b)
	assert_int(ship_a.clear_as_player_calls).is_equal(1)


## The client has never rendered ship 3 (never entered AoI), so there is no
## Node3D to attach the camera to -- this case is left alone (needs to spawn
## the ship first, docs/architecture/ownership.md §8), not a regression.
func test_switching_active_ship_to_an_unknown_ship_leaves_the_camera_alone() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)

	_main._ships = {11: ship_a}
	_dispatch_fixture("InitialState", 11)
	_main._player_ship_id = 11
	camera.set_target(ship_a)

	_dispatch_fixture("PlayerLoadoutUnknown", 11)
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(11)
	assert_object(camera._target_node).is_equal(ship_a)


func test_pending_docked_switch_reapplies_dock_after_aoi_enter() -> void:
	var old_ship := _setup_pending_docked_switch()

	_dispatch_to_main("AoiEnterPending", 11)

	assert_int(_main._session.player_ship_id()).is_equal(33)
	assert_int(_main._player_ship_id).is_equal(33)
	assert_bool(_main._session.is_docked()).is_true()
	assert_bool(_main._ships.has(33)).is_true()
	var new_ship := _main._ships[33] as FakeShip
	assert_int(new_ship.set_as_player_calls).is_equal(1)
	assert_int(new_ship.dock_calls.size()).is_equal(1)
	assert_int(new_ship.dock_calls[0]["tick"] as int).is_equal(13)
	assert_int(old_ship.clear_as_player_calls).is_equal(1)


func test_pending_docked_switch_reapplies_dock_after_ship_spawned() -> void:
	var old_ship := _setup_pending_docked_switch()

	_dispatch_to_main("ShipSpawnedPending", 11)

	assert_int(_main._session.player_ship_id()).is_equal(33)
	assert_int(_main._player_ship_id).is_equal(33)
	assert_bool(_main._session.is_docked()).is_true()
	assert_bool(_main._ships.has(33)).is_true()
	var new_ship := _main._ships[33] as FakeShip
	assert_int(new_ship.set_as_player_calls).is_equal(1)
	assert_int(new_ship.dock_calls.size()).is_equal(1)
	assert_int(new_ship.dock_calls[0]["tick"] as int).is_equal(13)
	assert_int(old_ship.clear_as_player_calls).is_equal(1)


## Disembarking is also applied by the typed PlayerLoadout outcome before the
## presentation side effect runs. The old ship must lose player-only visuals.
func test_disembarking_reverts_the_old_ships_player_material() -> void:
	var camera: Camera3D = auto_free(load("res://scripts/camera_controller.gd").new())
	add_child(camera)
	_main._camera = camera
	_main._presentation._camera = camera
	_main._hud_surface.build(
		auto_free(Node.new()), auto_free(CanvasLayer.new()), auto_free(Label.new()))

	var ship_a: FakeShip = auto_free(FakeShip.new())
	add_child(ship_a)
	_main._ships = {11: ship_a}
	_dispatch_fixture("InitialState", 11)
	_main._set_as_player_ship(11, ship_a)

	_dispatch_fixture("PlayerLoadoutDisembark", 11)
	_main._apply_loadout_side_effects()

	assert_int(_main._player_ship_id).is_equal(-1)
	assert_int(_main._session.player_ship_id()).is_equal(-1)
	assert_int(ship_a.clear_as_player_calls).is_equal(1)


func test_station_inventory_clicks_use_typed_actions() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.disassemble(), null, 0, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_click(row)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_station_inventory_build_and_disassemble_require_active_docked_context() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.disassemble(), null, 0, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_click(row)

	assert_int(connection.station_inventory_actions.size()).is_equal(0)
	connection.free()

	var undocked := FakeConnection.new()
	_main._connection = undocked
	_setup_docked_session()
	_main._session = WorldSession.new()
	_main._handle_inventory_row_click(row)
	assert_int(undocked.station_inventory_actions.size()).is_equal(0)
	undocked.free()


func test_shipless_docked_player_can_assemble_and_select_active_ship() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = -1
	_setup_docked_session()

	var packaged: ItemIdentity = ItemIdentity.packaged_ship(7) as ItemIdentity
	var assemble_row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.station(packaged), packaged, 1, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_click(assemble_row)
	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)

	var select_row: InventoryRow = InventoryRow.for_ship(
		null, 9, StationInventoryRow.owned_ship(9, false) as StationInventoryRow)
	_main._handle_inventory_row_click(select_row)
	assert_int(connection.station_inventory_actions.size()).is_equal(2)
	assert_int(connection.station_inventory_actions[1].request_count()).is_equal(1)
	connection.free()


func test_build_picker_is_local_and_ship_choice_is_typed() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	var toggle: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.build_toggle(), null, 0, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_click(toggle)
	assert_int(connection.station_inventory_actions.size()).is_equal(0)

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.build_ship_type(42) as StationInventoryRow,
		null, 0, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_click(row)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_unfit_all_row_click_sends_one_unfit_command_per_fitted_module() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	_set_loadout_modules([
		_module_fixture(1, "High", false),
		_module_fixture(2, "Low", false),
	])

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.unfit_all(), null, 0, InventoryRow.SOURCE_FITTED)
	_main._handle_inventory_row_click(row)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(2)
	connection.free()


func test_unfit_all_row_click_is_a_no_op_when_no_module_is_fitted() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.unfit_all(), null, 0, InventoryRow.SOURCE_FITTED)
	_main._handle_inventory_row_click(row)

	assert_int(connection.station_inventory_actions.size()).is_equal(0)
	connection.free()


# -- Drag-and-drop dispatch matrix --------------------------------------------

func test_drag_from_ship_cargo_to_fitted_sends_fit_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.cargo(ItemIdentity.module(5) as ItemIdentity, "High") as StationInventoryRow,
		ItemIdentity.module(5) as ItemIdentity, 1, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_FITTED, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_drag_from_ship_cargo_to_fitted_is_a_no_op_when_undocked() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.cargo(ItemIdentity.module(5) as ItemIdentity, "High") as StationInventoryRow,
		ItemIdentity.module(5) as ItemIdentity, 1, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_FITTED, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(0)
	connection.free()


func test_drag_from_fitted_to_ship_cargo_sends_unfit_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.fitted_with_index(5, "High", 0) as StationInventoryRow,
		null, 0, InventoryRow.SOURCE_FITTED)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_drag_from_ship_cargo_to_station_sends_transfer_to_station_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.cargo(ItemIdentity.scrap_metal(), "") as StationInventoryRow,
		ItemIdentity.scrap_metal(), 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_STATION, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_drag_from_station_to_ship_cargo_sends_transfer_from_station_command() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.station(ItemIdentity.scrap_metal()),
		ItemIdentity.scrap_metal(), 4, InventoryRow.SOURCE_STATION)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_drag_dropped_back_onto_its_own_column_is_a_no_op() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	var row: InventoryRow = InventoryRow.for_item(
		null, StationInventoryRow.cargo(ItemIdentity.scrap_metal(), "") as StationInventoryRow,
		ItemIdentity.scrap_metal(), 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._handle_inventory_row_drop(row, InventoryRow.SOURCE_SHIP_CARGO, Vector2.ZERO)

	assert_int(connection.station_inventory_actions.size()).is_equal(0)
	connection.free()


## Reordering needs the real built panel (inventory_panel_row_at() reads live
## Control rects), unlike the other drag cases above.
func test_drag_within_fitted_reorders_two_modules_of_the_same_slot_kind() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	var mid_1: ModuleRow = _module_fixture(1, "Mid", false)
	var mid_2: ModuleRow = _module_fixture(2, "Mid", false, "", 1)
	_set_loadout_modules([mid_1, mid_2])
	_main._hud_surface.set_player_fitting(_main._loadout.modules(), [])
	## inventory_panel_row_at() (used by the reorder branch) short-circuits
	## to null while the panel is hidden -- it starts hidden by default.
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var fitted_rows: Array[InventoryRow] = _main._hud_surface._inventory_panel_refs.fitted_rows
	var source_row: InventoryRow = fitted_rows[0]
	var target_row: InventoryRow = fitted_rows[1]
	var target_pos: Vector2 = (target_row.panel as Panel).get_global_rect().position + Vector2(2, 2)

	_main._handle_inventory_row_drop(source_row, InventoryRow.SOURCE_FITTED, target_pos)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_int(connection.station_inventory_actions[0].request_count()).is_equal(1)
	connection.free()


func test_drag_within_fitted_across_different_slot_kinds_is_a_no_op() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))

	_set_loadout_modules([
		_module_fixture(1, "High", false),
		_module_fixture(2, "Mid", false),
	])
	_main._hud_surface.set_player_fitting(_main._loadout.modules(), [])
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var fitted_rows: Array[InventoryRow] = _main._hud_surface._inventory_panel_refs.fitted_rows
	var source_row: InventoryRow = fitted_rows[0]
	var target_row: InventoryRow = fitted_rows[1]
	var target_pos: Vector2 = (target_row.panel as Panel).get_global_rect().position + Vector2(2, 2)

	_main._handle_inventory_row_drop(source_row, InventoryRow.SOURCE_FITTED, target_pos)

	assert_int(connection.station_inventory_actions.size()).is_equal(0)
	connection.free()


# -- Drag threshold (click vs. drop) -------------------------------------------

func test_release_within_threshold_of_press_is_treated_as_a_plain_click() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()

	_main._drag_row = InventoryRow.for_item(
		null, StationInventoryRow.fitted_with_index(5, "High", 0) as StationInventoryRow,
		null, 0, InventoryRow.SOURCE_FITTED)
	_main._drag_start_pos = Vector2(100, 100)
	_main._end_inventory_drag(Vector2(102, 101))  # well within DRAG_THRESHOLD_PX

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	assert_object(_main._drag_row).is_null()
	connection.free()


func test_release_past_threshold_is_treated_as_a_drop_not_a_click() -> void:
	var connection := FakeConnection.new()
	_main._connection = connection
	_main._player_ship_id = 1
	_setup_docked_session()
	var hud: CanvasLayer = auto_free(CanvasLayer.new())
	add_child(hud)
	_main._hud_surface.build(auto_free(Node.new()), hud, auto_free(Label.new()))
	HudManager.update_inventory_panel(_main._hud_surface._inventory_panel_refs, [], [], [], [], [])
	HudManager.toggle_inventory_panel(_main._hud_surface._inventory_panel_refs)
	await get_tree().process_frame

	var station_list: VBoxContainer = _main._hud_surface._inventory_panel_refs.station_list
	var far_pos: Vector2 = station_list.get_global_rect().position + Vector2(2, 2)

	_main._drag_row = InventoryRow.for_item(
		null, StationInventoryRow.cargo(ItemIdentity.scrap_metal(), "") as StationInventoryRow,
		ItemIdentity.scrap_metal(), 4, InventoryRow.SOURCE_SHIP_CARGO)
	_main._drag_start_pos = far_pos + Vector2(500, 500)  # far past DRAG_THRESHOLD_PX
	_main._end_inventory_drag(far_pos)

	assert_int(connection.station_inventory_actions.size()).is_equal(1)
	connection.free()


## Typed fixture builders for the navigation caches `main.gd` fills from
## `WorldSession.gates()`/`.stations()` (session_record_gd.rs).
func _gate(gate_id: int, pos: PackedFloat64Array, activation_radius: float, to_system_name: String) -> GateRecord:
	var g := GateRecord.new()
	g.gate_id = gate_id
	g.position = pos
	g.activation_radius = activation_radius
	g.to_system_name = to_system_name
	return g


func _station(station_id: int, name: String, pos: PackedFloat64Array, docking_radius: float) -> StationRecord:
	var st := StationRecord.new()
	st.station_id = station_id
	st.name = name
	st.position = pos
	st.docking_radius = docking_radius
	return st
