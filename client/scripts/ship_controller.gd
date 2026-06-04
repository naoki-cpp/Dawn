## ship_controller.gd
##
## 個々の Ship エンティティの表示・移動補間・速度インジケーターを担当する。

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

const WORLD_SCALE   : float = 0.1
const LERP_SPEED    : float = 8.0    ## 位置補間の速さ（delta 倍するので >1/delta でオーバーシュートに注意）
const VEL_VIS_SCALE : float = 4.0    ## 速度インジケーターの長さスケール

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int     = 0
var _target_pos   : Vector3 = Vector3.ZERO
var _vel_estimate : Vector3 = Vector3.ZERO   ## 推定速度ベクトル（Godot 座標/tick）
var _is_init      : bool    = false
var _is_player    : bool    = false

# 速度インジケーター（プレイヤー船のみ）
var _vel_mesh     : ImmediateMesh   = null
var _vel_instance : MeshInstance3D  = null

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos = position

func _process(delta: float) -> void:
	if not _is_init:
		return
	## clampf で補間係数を [0,1] に制限 → 低フレームレートでもオーバーシュートしない
	position = position.lerp(_target_pos, clampf(LERP_SPEED * delta, 0.0, 1.0))
	if _is_player and _vel_mesh != null:
		_draw_velocity_indicator()

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
	_setup_velocity_indicator()

## 現在の推定速度（サーバー座標系 units/tick）
func get_speed_server() -> float:
	return _vel_estimate.length() / WORLD_SCALE

# ── 速度インジケーター ────────────────────────────────────────────────────────

func _setup_velocity_indicator() -> void:
	_vel_mesh     = ImmediateMesh.new()
	_vel_instance = MeshInstance3D.new()
	_vel_instance.mesh = _vel_mesh

	var mat := StandardMaterial3D.new()
	mat.albedo_color               = Color(0.0, 1.0, 0.4, 1.0)
	mat.emission_enabled           = true
	mat.emission                   = Color(0.0, 1.0, 0.4, 1.0)
	mat.emission_energy_multiplier = 3.0
	mat.shading_mode               = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = false   ## surface_set_color を使わないので false
	mat.cull_mode                  = BaseMaterial3D.CULL_DISABLED
	_vel_instance.set_surface_override_material(0, mat)

	add_child(_vel_instance)

func _draw_velocity_indicator() -> void:
	_vel_mesh.clear_surfaces()

	var speed: float = _vel_estimate.length()
	if speed < 0.001:
		return

	var dir: Vector3 = _vel_estimate / speed          ## 正規化（除算で NaN 回避）
	var tip: Vector3 = dir * speed * VEL_VIS_SCALE    ## 先端位置

	## 矢頭用の垂直ベクトルを求める（dir が UP と平行な場合は RIGHT を使う）
	var ref_up: Vector3 = Vector3.UP
	if absf(dir.dot(Vector3.UP)) > 0.9:
		ref_up = Vector3.RIGHT
	var perp: Vector3 = dir.cross(ref_up).normalized()

	var arrow_len: float = minf(speed * VEL_VIS_SCALE * 0.2, 80.0)

	_vel_mesh.surface_begin(Mesh.PRIMITIVE_LINES)

	## 本線
	_vel_mesh.surface_add_vertex(Vector3.ZERO)
	_vel_mesh.surface_add_vertex(tip)

	## 矢頭（4本）
	for i in 4:
		var angle: float = i * PI / 2.0
		var wing: Vector3 = perp.rotated(dir, angle) * arrow_len
		_vel_mesh.surface_add_vertex(tip)
		_vel_mesh.surface_add_vertex(tip - dir * arrow_len * 2.0 + wing)

	_vel_mesh.surface_end()

# ── 座標変換 ─────────────────────────────────────────────────────────────────

static func _to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.y, -v.z) * WORLD_SCALE
