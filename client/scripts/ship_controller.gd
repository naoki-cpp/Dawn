## ship_controller.gd
##
## 個々の Ship の表示・移動補間・各種ビジュアルエフェクトを担当する。
##
## 矢印の種類:
##   緑   : 速度ベクトル（大きさ ∝ speed）
##   橙   : 推力方向ベクトル（固定長・thrust が設定されている間だけ表示）
##
## エフェクト:
##   白フラッシュ : ロックオン選択時
##   赤フラッシュ : ダメージ被弾時
##   シアン枠線  : ロック中（点滅）
##   シアン枠線  : ロック完了（常時点灯）
##   爆発リング  : 破壊時

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

const WORLD_SCALE     : float = 0.1
const LERP_SPEED      : float = 8.0
const VEL_VIS_SCALE   : float = 4.0
const THRUST_VIS_LEN  : float = 350.0

## ロック枠線のサイズ（Godot units、破壊エフェクトの膨張リング起点に使用）
const LOCK_RING_RADIUS: float = 250.0

## ロック中/ロック完了インジケーターの画面上サイズ。navigation_marker_renderer.gd
## の惑星選択リング（RETICLE_PIXEL_SIZE）と揃える -- 同じ「選択/状態インジケーター」
## なので見た目・距離耐性も揃える（BillboardRing 経由で fixed_size ビルボード化）。
const LOCK_RING_PIXEL_SIZE: float = 0.0015

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int     = 0
var _velocity     : Vector3 = Vector3.ZERO  ## Godot 座標系の速度（VelocityChanged から）
var _vel_estimate : Vector3 = Vector3.ZERO  ## 速度インジケーター用（互換）
var _is_init      : bool    = false
var _is_player    : bool    = false
var _thrust_dir   : Vector3 = Vector3.ZERO
var _motion       := MotionPredictor.new()
var _motion_ready : bool    = false
## Hidden while warping faster than VISUAL_SPEED_CAP (ADR-0029 lore pass,
## non-player ships only). Tracks the last applied visibility so we only
## toggle `visible` on the actual crossing, not every frame.
var _in_tunnel    : bool    = false

## ロック状態: "none" / "locking" / "locked"
var _lock_state   : String  = "none"
var _lock_blink_t : float   = 0.0  ## 点滅タイマー

# 速度インジケーター（緑）
var _vel_mesh     : ImmediateMesh  = null
var _vel_instance : MeshInstance3D = null

# 推力インジケーター（橙）
var _thr_mesh     : ImmediateMesh  = null
var _thr_instance : MeshInstance3D = null

# ロック枠線（シアン、fixed_size ビルボード -- BillboardRing 参照）
var _lock_instance: Sprite3D = null

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	## 全 Ship に枠線インジケーターを生成（非表示で待機）
	_lock_instance = BillboardRing.build(Color(0.0, 1.0, 1.0), LOCK_RING_PIXEL_SIZE)  ## シアン
	add_child(_lock_instance)
	_lock_instance.visible = false

const TICKS_PER_SEC    : float = 10.0  ## Server tick rate
const ROT_THRESHOLD_SQ : float = 0.02  ## Min speed² before rotating (Godot units²/frame²)
const ROT_SPEED        : float = 4.0   ## Slerp speed toward velocity direction
## Cap on the VISUAL integration speed (Godot units/tick). A true-AU warp runs at
## ~10^8 Godot units/tick; integrating that literally flings the ship past the
## far plane and jitters in f32 (motion sickness). We instead streak forward at a
## bounded, renderable speed and let the authoritative PositionSnap (ADR-0029)
## place the ship at the real arrival point. The true speed is still shown on the
## HUD (get_speed_server uses the uncapped _velocity).
const VISUAL_SPEED_CAP : float = 2_000.0

func _process(delta: float) -> void:
	if not _is_init:
		return

	if not _motion_ready:
		return

	## Both local prediction and remote dead reckoning advance through the same
	## Rust motion track. This adapter only decides how much of that state is
	## renderable for the current ship.
	_motion.advance(delta * TICKS_PER_SEC)
	_velocity = _motion.predicted_velocity()
	var speed := _velocity.length()
	var predicted_pos := _motion.predicted_position()
	if not _is_player:
		## Remote ships are hidden during the bulk of a warp while their shared
		## track continues toward the true position.
		var in_tunnel := speed > VISUAL_SPEED_CAP
		if in_tunnel != _in_tunnel:
			visible    = not in_tunnel
			_in_tunnel = in_tunnel
	position = predicted_pos
	_vel_estimate = _velocity

	## Rotate the ship to face its velocity direction.
	## The Hull mesh tip is in local -Z after its -90° X rotation, which
	## aligns with Node3D.look_at() convention (-Z = forward).
	if _velocity.length_squared() > ROT_THRESHOLD_SQ:
		var vel_dir := _velocity.normalized()
		var up      := Vector3.UP if absf(vel_dir.dot(Vector3.UP)) < 0.95 else Vector3.RIGHT
		var target  := Basis.looking_at(vel_dir, up)
		basis        = basis.slerp(target, minf(delta * ROT_SPEED, 1.0))

	## Player indicators: transform world-space directions to local space
	## so they draw correctly even after the root node has rotated.
	if _is_player:
		var inv := basis.transposed()  ## Inverse of an orthonormal basis
		if _vel_mesh != null:
			var local_vel := inv * _vel_estimate
			_draw_arrow(_vel_mesh, local_vel, local_vel.length() * VEL_VIS_SCALE)
		if _thr_mesh != null:
			var local_thr := inv * _thrust_dir
			_draw_arrow(_thr_mesh, local_thr, THRUST_VIS_LEN if _thrust_dir.length_squared() > 0.0 else 0.0)

	_update_lock_indicator(delta)

# ── 公開 API ──────────────────────────────────────────────────────────────────

## godot_pos is already in Godot world space (main.gd's WorldSpace converts at
## the boundary, applying the floating origin). Ship nodes stay origin-agnostic.
func initialize(id: int, godot_pos: Vector3) -> void:
	ship_id    = id
	position   = godot_pos
	_velocity  = Vector3.ZERO
	_is_init   = true
	_in_tunnel = false
	visible    = true
	_motion.configure_dead_reckoning(
		500.0 * WORLD_SCALE, 10_000_000.0, 0.3, godot_pos, Vector3.ZERO, 0)
	_motion_ready = true

## Seed the Rust predictor with the server's effective fitted movement profile.
## The predictor runs in local Godot units so it owns the same position that is
## rendered by this node.
func configure_motion(max_speed: float, mass: float, inertia_modifier: float, server_vel: Vector3, tick: int = 0) -> void:
	_motion.configure_dead_reckoning(
		max_speed * WORLD_SCALE,
		mass,
		inertia_modifier,
		global_position,
		_server_velocity_to_godot(server_vel),
		tick)
	_motion_ready = true
	_velocity = _server_velocity_to_godot(server_vel)

## VelocityChanged イベントで速度を更新する（ADR-0008）。
## server_vel はサーバー座標系の速度ベクトル（units/tick）。
func set_velocity(server_vel: Vector3) -> void:
	_velocity = _server_velocity_to_godot(server_vel)
	if _motion_ready:
		## Warp velocity is not governed by the normal fitted movement profile.
		## The shared Rust track owns the capped presentation until PositionSnap
		## resets it.
		if _is_player and _velocity.length() > VISUAL_SPEED_CAP:
			_motion.begin_warp(VISUAL_SPEED_CAP)
		_motion.set_velocity(_velocity)

## Snap the ship to a Godot-space position (jump-gate teleport, warp-arrival
## snap). main.gd converts from server space via its WorldSpace before calling.
func update_target(godot_pos: Vector3, tick: int = 0) -> void:
	reset_motion(godot_pos, Vector3.ZERO, tick)

## Shift the Rust motion track with the rendered node during a floating-origin
## rebase. The node position itself is moved by WorldPresentation.
func rebase_motion(shift: Vector3) -> void:
	if _motion_ready:
		_motion.rebase(shift)

func set_as_player() -> void:
	_is_player    = true
	if _motion_ready:
		## A reconnect can materialize a ship that is already in committed warp,
		## before any later VelocityChanged event gives us the high-speed hint.
		## Preserve dead reckoning in that case instead of applying the normal
		## fitted movement profile to the warp velocity on the next tick.
		if _velocity.length() > VISUAL_SPEED_CAP:
			_motion.begin_warp(VISUAL_SPEED_CAP)
		else:
			_motion.enable_prediction()
	_vel_instance = _make_indicator(Color(0.0, 1.0, 0.4))
	_vel_mesh     = _vel_instance.mesh as ImmediateMesh
	_thr_instance = _make_indicator(Color(1.0, 0.55, 0.0))
	_thr_mesh     = _thr_instance.mesh as ImmediateMesh

## Undo set_as_player() when a different ship becomes the piloted one
## (ADR-0037 SelectActiveShip/Disembark/Assemble). Frees the velocity/thrust
## indicator MeshInstances rather than just flipping _is_player off --
## leaving them around would freeze their last-drawn arrows in place forever
## since _process() only redraws them while _is_player is true.
func clear_as_player() -> void:
	if _motion_ready:
		_motion.enable_dead_reckoning()
	_is_player = false
	if _vel_instance != null:
		_vel_instance.queue_free()
		_vel_instance = null
		_vel_mesh = null
	if _thr_instance != null:
		_thr_instance.queue_free()
		_thr_instance = null
		_thr_mesh = null

## Apply local prediction input for a MoveCommand.
func set_thrust_direction(godot_dir: Vector3) -> void:
	_thrust_dir = godot_dir.normalized() if godot_dir.length_squared() > 0.0 else Vector3.ZERO
	if _motion_ready and _is_player:
		if _thrust_dir.length_squared() > 0.0:
			_motion.set_thrust_direction(_thrust_dir)
		else:
			_motion.clear_input()

## Apply local prediction input for a StopCommand.
func set_braking() -> void:
	_thrust_dir = Vector3.ZERO
	if _motion_ready and _is_player:
		_motion.set_braking()

## Reconcile normal flight to an authoritative state without discarding local
## input. The Rust module ignores stale ticks.
func reconcile_motion(godot_pos: Vector3, server_vel: Vector3, tick: int) -> void:
	var godot_vel := _server_velocity_to_godot(server_vel)
	_velocity = godot_vel
	if _motion_ready and _is_player:
		## MotionPredictor keeps the authoritative base and smooths the visual
		## correction. Writing global_position here would undo that smoothing and
		## make every delayed correction visibly snap backwards.
		_motion.reconcile(godot_pos, godot_vel, tick)
	else:
		global_position = godot_pos

## Reset prediction at a discontinuity such as warp arrival or docking.
func reset_motion(godot_pos: Vector3, server_vel: Vector3, tick: int) -> void:
	var godot_vel := _server_velocity_to_godot(server_vel)
	global_position = godot_pos
	_velocity = godot_vel
	_thrust_dir = Vector3.ZERO
	if _motion_ready:
		_motion.reset(godot_pos, godot_vel, tick)
		if _is_player:
			_motion.enable_prediction()
		else:
			_motion.enable_dead_reckoning()

## Enter the explicit docked state. The Rust track owns the zero-velocity and
## no-integration invariant; this adapter applies the authoritative position.
func dock_motion(godot_pos: Vector3, tick: int) -> void:
	global_position = godot_pos
	_velocity = Vector3.ZERO
	_thrust_dir = Vector3.ZERO
	if _motion_ready:
		_motion.dock(godot_pos, tick)

## Leave the explicit docked state and choose prediction/dead-reckoning based
## on which ship owns this controller.
func undock_motion(godot_pos: Vector3, server_vel: Vector3, tick: int) -> void:
	var godot_vel := _server_velocity_to_godot(server_vel)
	global_position = godot_pos
	_velocity = godot_vel
	_thrust_dir = Vector3.ZERO
	if _motion_ready:
		_motion.undock(godot_pos, godot_vel, tick, _is_player)

func get_speed_server() -> float:
	return _velocity.length() / WORLD_SCALE

## Godot-space speed (i.e. already * WORLD_SCALE, same units as
## VISUAL_SPEED_CAP). Used by main.gd to drive the warp-tunnel overlay
## (ADR-0029 lore pass) -- comparable to VISUAL_SPEED_CAP without re-deriving
## the scale conversion at the call site.
func get_speed_godot() -> float:
	return _velocity.length()

func _server_velocity_to_godot(server_vel: Vector3) -> Vector3:
	return Vector3(server_vel.x, server_vel.y, -server_vel.z) * WORLD_SCALE

## ロック状態を設定する。
## state: "none" / "locking" / "locked"
func set_lock_state(state: String) -> void:
	_lock_state   = state
	_lock_blink_t = 0.0
	if _lock_instance != null:
		_lock_instance.modulate.a = 1.0  ## reset breathing alpha from a previous "locked" phase
	_lock_instance.visible = (state != "none")

## ロックオン選択時に一瞬白く光らせる（視覚フィードバック）。
func flash_lock_indicator() -> void:
	_flash_hull(Color(1.0, 1.0, 1.0), Color(1.0, 1.0, 1.0), 0.15)

## ダメージ被弾時に赤くフラッシュさせる。
func flash_damage() -> void:
	_flash_hull(Color(1.0, 0.1, 0.1), Color(1.0, 0.0, 0.0), 0.1)

func flash_repair() -> void:
	_flash_hull(Color(0.15, 1.0, 0.45), Color(0.0, 1.0, 0.25), 0.12)

## 破壊エフェクト（膨張リング）を再生して自分自身を削除する。
func play_destroy_effect() -> void:
	## Hull を非表示にして爆発リングだけ残す
	var hull: MeshInstance3D = get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.visible = false
	var light: OmniLight3D = get_node_or_null("EngineLight") as OmniLight3D
	if light != null:
		light.visible = false
	var glow: MeshInstance3D = get_node_or_null("EngineGlow") as MeshInstance3D
	if glow != null:
		glow.visible = false
	_lock_instance.visible = false

	## 膨張するリングを描画しながらフェードアウト
	var ring_inst: MeshInstance3D = _make_line_indicator(Color(1.0, 0.5, 0.0))
	var ring_mesh: ImmediateMesh  = ring_inst.mesh as ImmediateMesh
	var ring_mat : Material = ring_inst.get_surface_override_material(0)

	var elapsed: float = 0.0
	var duration: float = 0.6
	while elapsed < duration:
		elapsed += get_process_delta_time()
		var t    : float = elapsed / duration
		var r    : float = LOCK_RING_RADIUS * (1.0 + t * 6.0)
		var alpha: float = 1.0 - t
		if ring_mat is StandardMaterial3D:
			(ring_mat as StandardMaterial3D).albedo_color.a              = alpha
			(ring_mat as StandardMaterial3D).emission_energy_multiplier  = (1.0 - t) * 4.0
		_draw_ring(ring_mesh, r)
		await get_tree().process_frame

	queue_free()

# ── ロック枠線の更新 ──────────────────────────────────────────────────────────

func _update_lock_indicator(delta: float) -> void:
	if _lock_state == "none" or _lock_instance == null:
		return

	_lock_blink_t += delta

	if _lock_state == "locking":
		## 点滅: 0.4 秒周期
		_lock_instance.visible = (fmod(_lock_blink_t, 0.4) < 0.2)
	elif _lock_state == "locked":
		## 常時点灯 + ブリージング（じんわり明滅、alpha で表現）
		_lock_instance.visible = true
		_lock_instance.modulate.a = clampf(0.55 + sin(_lock_blink_t * 3.0) * 0.45, 0.0, 1.0)

# ── ハルフラッシュ共通処理 ───────────────────────────────────────────────────

func _flash_hull(color: Color, emission: Color, duration: float) -> void:
	var hull: MeshInstance3D = get_node_or_null("Hull") as MeshInstance3D
	if hull == null:
		return
	var orig_mat: Material = hull.get_surface_override_material(0)
	var flash_mat := StandardMaterial3D.new()
	flash_mat.albedo_color               = color
	flash_mat.emission_enabled           = true
	flash_mat.emission                   = emission
	flash_mat.emission_energy_multiplier = 6.0
	flash_mat.shading_mode               = BaseMaterial3D.SHADING_MODE_UNSHADED
	hull.set_surface_override_material(0, flash_mat)
	await get_tree().create_timer(duration).timeout
	if is_instance_valid(hull):
		hull.set_surface_override_material(0, orig_mat)

# ── リング描画 ────────────────────────────────────────────────────────────────

func _draw_ring(mesh: ImmediateMesh, radius: float) -> void:
	mesh.clear_surfaces()
	mesh.surface_begin(Mesh.PRIMITIVE_LINES)
	var segments: int = 24
	for i: int in segments:
		var a0: float = (i       as float) / segments * TAU
		var a1: float = ((i + 1) as float) / segments * TAU
		mesh.surface_add_vertex(Vector3(cos(a0) * radius, 0.0, sin(a0) * radius))
		mesh.surface_add_vertex(Vector3(cos(a1) * radius, 0.0, sin(a1) * radius))
	mesh.surface_end()

# ── 矢印描画 ──────────────────────────────────────────────────────────────────

func _draw_arrow(mesh: ImmediateMesh, dir: Vector3, length: float) -> void:
	mesh.clear_surfaces()
	if length < 0.5 or dir.length_squared() < 0.001:
		return
	var unit: Vector3 = dir.normalized()
	var tip : Vector3 = unit * length
	var ref : Vector3 = Vector3.UP if absf(unit.dot(Vector3.UP)) < 0.9 else Vector3.RIGHT
	var perp: Vector3 = unit.cross(ref).normalized()
	var head: float   = minf(length * 0.2, 80.0)
	mesh.surface_begin(Mesh.PRIMITIVE_LINES)
	mesh.surface_add_vertex(Vector3.ZERO)
	mesh.surface_add_vertex(tip)
	for i: int in 4:
		var wing: Vector3 = perp.rotated(unit, i * PI * 0.5) * head
		mesh.surface_add_vertex(tip)
		mesh.surface_add_vertex(tip - unit * head * 2.0 + wing)
	mesh.surface_end()

# ── インジケーター生成 ────────────────────────────────────────────────────────

func _make_indicator(color: Color) -> MeshInstance3D:
	return _make_line_indicator(color)

func _make_line_indicator(color: Color) -> MeshInstance3D:
	var imesh := ImmediateMesh.new()
	var inst  := MeshInstance3D.new()
	inst.mesh = imesh
	var mat := StandardMaterial3D.new()
	mat.albedo_color               = color
	mat.emission_enabled           = true
	mat.emission                   = color
	mat.emission_energy_multiplier = 3.0
	mat.shading_mode               = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = false
	mat.cull_mode                  = BaseMaterial3D.CULL_DISABLED
	inst.material_override = mat
	add_child(inst)
	return inst
