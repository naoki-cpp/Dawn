## ship_controller.gd
##
## 個々の Ship エンティティの表示と移動補間を担当する。

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

const WORLD_SCALE : float = 0.1
const LERP_SPEED  : float = 8.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id    : int     = 0
var _target_pos: Vector3 = Vector3.ZERO
var _is_init   : bool    = false

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos = position

func _process(delta: float) -> void:
	if not _is_init:
		return
	position = position.lerp(_target_pos, LERP_SPEED * delta)

# ── 公開 API ──────────────────────────────────────────────────────────────────

func initialize(id: int, server_pos: Vector3) -> void:
	ship_id     = id
	_target_pos = _to_godot(server_pos)
	position    = _target_pos
	_is_init    = true

func update_target(server_pos: Vector3) -> void:
	_target_pos = _to_godot(server_pos)

# ── 座標変換 ─────────────────────────────────────────────────────────────────

static func _to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.y, -v.z) * WORLD_SCALE
