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

## ロック枠線のサイズ（Godot units）
const LOCK_RING_RADIUS: float = 250.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int     = 0
var _velocity     : Vector3 = Vector3.ZERO  ## Godot 座標系の速度（VelocityChanged から）
var _vel_estimate : Vector3 = Vector3.ZERO  ## 速度インジケーター用（互換）
var _is_init      : bool    = false
var _is_player    : bool    = false
var _thrust_dir   : Vector3 = Vector3.ZERO

## ロック状態: "none" / "locking" / "locked"
var _lock_state   : String  = "none"
var _lock_blink_t : float   = 0.0  ## 点滅タイマー

# 速度インジケーター（緑）
var _vel_mesh     : ImmediateMesh  = null
var _vel_instance : MeshInstance3D = null

# 推力インジケーター（橙）
var _thr_mesh     : ImmediateMesh  = null
var _thr_instance : MeshInstance3D = null

# ロック枠線（シアン）
var _lock_mesh    : ImmediateMesh  = null
var _lock_instance: MeshInstance3D = null

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos = position
	## 全 Ship に枠線インジケーターを生成（非表示で待機）
	_lock_instance = _make_line_indicator(Color(0.0, 1.0, 1.0))  ## シアン
	_lock_mesh     = _lock_instance.mesh as ImmediateMesh
	_lock_instance.visible = false

const TICKS_PER_SEC : float = 10.0  ## サーバーの Tick レート

func _process(delta: float) -> void:
	if not _is_init:
		return

	## VelocityChanged (ADR-0008): フレームごとに velocity で位置を更新する
	## Tick 間を補間するため、Tick 境界でのジャンプがなくなる
	position += _velocity * delta * TICKS_PER_SEC
	_vel_estimate = _velocity  ## 速度インジケーター用に同期

	## プレイヤー船の矢印描画
	if _is_player:
		if _vel_mesh != null:
			_draw_arrow(_vel_mesh, _vel_estimate, _vel_estimate.length() * VEL_VIS_SCALE)
		if _thr_mesh != null:
			_draw_arrow(_thr_mesh, _thrust_dir, THRUST_VIS_LEN if _thrust_dir.length_squared() > 0.0 else 0.0)

	## ロック枠線の更新
	_update_lock_indicator(delta)

# ── 公開 API ──────────────────────────────────────────────────────────────────

func initialize(id: int, server_pos: Vector3) -> void:
	ship_id   = id
	position  = _to_godot(server_pos)
	_velocity = Vector3.ZERO
	_is_init  = true

## VelocityChanged イベントで速度を更新する（ADR-0008）。
## server_vel はサーバー座標系の速度ベクトル（units/tick）。
func set_velocity(server_vel: Vector3) -> void:
	## サーバー座標系 → Godot 座標系（Z 反転・スケール変換）
	_velocity = Vector3(server_vel.x, server_vel.y, -server_vel.z) * WORLD_SCALE

## 後方互換のため残す（InitialState での位置設定に使う）。
func update_target(server_pos: Vector3) -> void:
	position = _to_godot(server_pos)

func set_as_player() -> void:
	_is_player    = true
	_vel_instance = _make_indicator(Color(0.0, 1.0, 0.4))
	_vel_mesh     = _vel_instance.mesh as ImmediateMesh
	_thr_instance = _make_indicator(Color(1.0, 0.55, 0.0))
	_thr_mesh     = _thr_instance.mesh as ImmediateMesh

func set_thrust_direction(godot_dir: Vector3) -> void:
	_thrust_dir = godot_dir.normalized() if godot_dir.length_squared() > 0.0 else Vector3.ZERO

func get_speed_server() -> float:
	return _velocity.length() / WORLD_SCALE

## ロック状態を設定する。
## state: "none" / "locking" / "locked"
func set_lock_state(state: String) -> void:
	_lock_state   = state
	_lock_blink_t = 0.0
	_lock_instance.visible = (state != "none")

## ロックオン選択時に一瞬白く光らせる（視覚フィードバック）。
func flash_lock_indicator() -> void:
	_flash_hull(Color(1.0, 1.0, 1.0), Color(1.0, 1.0, 1.0), 0.15)

## ダメージ被弾時に赤くフラッシュさせる。
func flash_damage() -> void:
	_flash_hull(Color(1.0, 0.1, 0.1), Color(1.0, 0.0, 0.0), 0.1)

## 破壊エフェクト（膨張リング）を再生して自分自身を削除する。
func play_destroy_effect() -> void:
	## Hull を非表示にして爆発リングだけ残す
	var hull: MeshInstance3D = get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.visible = false
	var light: OmniLight3D = get_node_or_null("EngineGlow") as OmniLight3D
	if light != null:
		light.visible = false
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
	if _lock_state == "none" or _lock_mesh == null:
		return

	_lock_blink_t += delta

	if _lock_state == "locking":
		## 点滅: 0.4 秒周期
		_lock_instance.visible = (fmod(_lock_blink_t, 0.4) < 0.2)
		_draw_ring(_lock_mesh, LOCK_RING_RADIUS)
	elif _lock_state == "locked":
		## 常時点灯
		_lock_instance.visible = true
		## ブリージング（じんわり明滅）
		var mat: StandardMaterial3D = _lock_instance.get_surface_override_material(0) as StandardMaterial3D
		if mat != null:
			mat.emission_energy_multiplier = 2.0 + sin(_lock_blink_t * 3.0) * 1.0
		_draw_ring(_lock_mesh, LOCK_RING_RADIUS)

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
	inst.set_surface_override_material(0, mat)
	add_child(inst)
	return inst

# ── 座標変換 ─────────────────────────────────────────────────────────────────

static func _to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.y, -v.z) * WORLD_SCALE
