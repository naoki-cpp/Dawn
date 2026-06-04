## ship_controller.gd
##
## 個々の Ship エンティティの表示・移動補間・速度インジケーターを担当する。

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

const WORLD_SCALE   : float = 0.1
const LERP_SPEED    : float = 12.0   ## 位置補間の速さ
const VEL_VIS_SCALE : float = 3.0    ## 速度インジケーターの長さスケール

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int     = 0
var _target_pos   : Vector3 = Vector3.ZERO
var _prev_target  : Vector3 = Vector3.ZERO   ## 1 tick 前のターゲット
var _vel_estimate : Vector3 = Vector3.ZERO   ## 推定速度ベクトル（Godot 座標）
var _is_init      : bool    = false
var _is_player    : bool    = false

# 速度インジケーター用 ImmediateMesh（プレイヤー船のみ）
var _vel_mesh     : ImmediateMesh    = null
var _vel_instance : MeshInstance3D  = null
var _vel_material : StandardMaterial3D = null

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos  = position
	_prev_target = position

func _process(delta: float) -> void:
	if not _is_init:
		return
	position = position.lerp(_target_pos, LERP_SPEED * delta)
	if _is_player:
		_draw_velocity_indicator()

# ── 公開 API ──────────────────────────────────────────────────────────────────

## SpawnedEvent で Ship を初期化する。
func initialize(id: int, server_pos: Vector3) -> void:
	ship_id      = id
	_target_pos  = _to_godot(server_pos)
	_prev_target = _target_pos
	position     = _target_pos
	_is_init     = true

## ShipMoved イベントでターゲット座標を更新する。
## 同時に速度を推定する（連続する ShipMoved の位置差分）。
func update_target(server_pos: Vector3) -> void:
	var new_target: Vector3 = _to_godot(server_pos)
	_vel_estimate = new_target - _target_pos   ## Godot 座標での1 Tick 変位
	_prev_target  = _target_pos
	_target_pos   = new_target

## プレイヤー船として指定する。速度インジケーターを有効化。
func set_as_player() -> void:
	_is_player = true
	_setup_velocity_indicator()

## 現在の推定速度（サーバー座標系 units/tick）を返す。
## HUD 表示に使用する。
func get_speed_server() -> float:
	return _vel_estimate.length() / WORLD_SCALE

# ── 速度インジケーター ────────────────────────────────────────────────────────

func _setup_velocity_indicator() -> void:
	_vel_mesh     = ImmediateMesh.new()
	_vel_instance = MeshInstance3D.new()
	_vel_instance.mesh = _vel_mesh

	_vel_material = StandardMaterial3D.new()
	_vel_material.albedo_color               = Color(0.0, 1.0, 0.4, 1)
	_vel_material.emission_enabled           = true
	_vel_material.emission                   = Color(0.0, 1.0, 0.4, 1)
	_vel_material.emission_energy_multiplier = 3.0
	_vel_material.shading_mode               = BaseMaterial3D.SHADING_MODE_UNSHADED
	_vel_material.no_depth_test              = false
	_vel_instance.set_surface_override_material(0, _vel_material)

	add_child(_vel_instance)

func _draw_velocity_indicator() -> void:
	if _vel_mesh == null:
		return

	_vel_mesh.clear_surfaces()

	var speed: float = _vel_estimate.length()
	if speed < 0.01:
		return

	## 速度ベクトルの長さを VEL_VIS_SCALE 倍して表示
	var tip: Vector3 = _vel_estimate.normalized() * speed * VEL_VIS_SCALE

	_vel_mesh.surface_begin(Mesh.PRIMITIVE_LINES)

	## 本線: 原点 → 先端
	_vel_mesh.surface_set_color(Color(0.0, 1.0, 0.4, 1))
	_vel_mesh.surface_add_vertex(Vector3.ZERO)
	_vel_mesh.surface_add_vertex(tip)

	## 矢頭（3本の短い線）
	var arrow_size: float = minf(speed * VEL_VIS_SCALE * 0.15, 60.0)
	var perp1: Vector3 = _vel_estimate.cross(Vector3.UP).normalized() * arrow_size
	var perp2: Vector3 = _vel_estimate.cross(perp1).normalized() * arrow_size
	for p in [perp1, -perp1, perp2, -perp2]:
		_vel_mesh.surface_add_vertex(tip)
		_vel_mesh.surface_add_vertex(tip - _vel_estimate.normalized() * arrow_size * 2 + p)

	_vel_mesh.surface_end()

# ── 座標変換 ─────────────────────────────────────────────────────────────────

static func _to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.y, -v.z) * WORLD_SCALE
