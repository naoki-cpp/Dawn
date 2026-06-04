## ship_controller.gd
##
## 個々の Ship の表示・移動補間・速度インジケーター・推力インジケーターを担当する。
##
## 矢印の種類:
##   緑   : 速度ベクトル（大きさ ∝ speed）
##   橙   : 推力方向ベクトル（固定長・thrust が設定されている間だけ表示）

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

const WORLD_SCALE     : float = 0.1
const LERP_SPEED      : float = 8.0

## 速度矢印: 長さスケール（速度 × このスケール）
const VEL_VIS_SCALE   : float = 4.0
## 推力矢印: 固定長（Godot 単位）
const THRUST_VIS_LEN  : float = 350.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int     = 0
var _target_pos   : Vector3 = Vector3.ZERO
var _vel_estimate : Vector3 = Vector3.ZERO
var _is_init      : bool    = false
var _is_player    : bool    = false

## 推力方向（Godot 座標系の単位ベクトル）。ZERO なら表示しない。
var _thrust_dir   : Vector3 = Vector3.ZERO

# 速度インジケーター（緑）
var _vel_mesh     : ImmediateMesh  = null
var _vel_instance : MeshInstance3D = null

# 推力インジケーター（橙）
var _thr_mesh     : ImmediateMesh  = null
var _thr_instance : MeshInstance3D = null

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos = position

func _process(delta: float) -> void:
	if not _is_init:
		return
	position = position.lerp(_target_pos, clampf(LERP_SPEED * delta, 0.0, 1.0))
	if _is_player:
		if _vel_mesh != null:
			_draw_arrow(_vel_mesh, _vel_estimate, _vel_estimate.length() * VEL_VIS_SCALE)
		if _thr_mesh != null:
			_draw_arrow(_thr_mesh, _thrust_dir, THRUST_VIS_LEN if _thrust_dir.length_squared() > 0.0 else 0.0)

# ── 公開 API ──────────────────────────────────────────────────────────────────

func initialize(id: int, server_pos: Vector3) -> void:
	ship_id     = id
	_target_pos = _to_godot(server_pos)
	position    = _target_pos
	_is_init    = true

func update_target(server_pos: Vector3) -> void:
	var new_target: Vector3 = _to_godot(server_pos)
	_vel_estimate = new_target - _target_pos
	_target_pos   = new_target

func set_as_player() -> void:
	_is_player = true
	_vel_instance = _make_indicator(Color(0.0, 1.0, 0.4))   ## 緑
	_vel_mesh     = _vel_instance.mesh as ImmediateMesh
	_thr_instance = _make_indicator(Color(1.0, 0.55, 0.0))  ## 橙
	_thr_mesh     = _thr_instance.mesh as ImmediateMesh

## 推力方向を設定する（Godot 座標系の方向ベクトル）。
## main.gd からダブルクリック時に呼ばれる。
func set_thrust_direction(godot_dir: Vector3) -> void:
	_thrust_dir = godot_dir.normalized() if godot_dir.length_squared() > 0.0 else Vector3.ZERO

## 現在の推定速度（サーバー座標系 units/tick）
func get_speed_server() -> float:
	return _vel_estimate.length() / WORLD_SCALE

# ── 矢印描画（共通ロジック） ──────────────────────────────────────────────────

func _draw_arrow(mesh: ImmediateMesh, dir: Vector3, length: float) -> void:
	mesh.clear_surfaces()

	if length < 0.5 or dir.length_squared() < 0.001:
		return

	var unit: Vector3 = dir.normalized()
	var tip : Vector3 = unit * length

	## 矢頭の垂直ベクトル（dir と UP が平行なら RIGHT を使う）
	var ref : Vector3 = Vector3.UP if absf(unit.dot(Vector3.UP)) < 0.9 else Vector3.RIGHT
	var perp: Vector3 = unit.cross(ref).normalized()
	var head: float   = minf(length * 0.2, 80.0)

	mesh.surface_begin(Mesh.PRIMITIVE_LINES)

	## 本線
	mesh.surface_add_vertex(Vector3.ZERO)
	mesh.surface_add_vertex(tip)

	## 矢頭（4本の羽）
	for i: int in 4:
		var wing: Vector3 = perp.rotated(unit, i * PI * 0.5) * head
		mesh.surface_add_vertex(tip)
		mesh.surface_add_vertex(tip - unit * head * 2.0 + wing)

	mesh.surface_end()

# ── インジケーター生成 ────────────────────────────────────────────────────────

func _make_indicator(color: Color) -> MeshInstance3D:
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
