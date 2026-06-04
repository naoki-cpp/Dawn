## camera_controller.gd
##
## プレイヤー船を追従するストラテジックカメラ。
##
## 操作:
##   左ボタンドラッグ : カメラを軌道回転（ターゲット中心）
##   マウスホイール   : ズームイン / ズームアウト

extends Camera3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

@export var follow_speed   : float = 4.0    ## 追従補間速さ
@export var zoom_min       : float = 300.0
@export var zoom_max       : float = 8000.0
@export var zoom_step      : float = 250.0
@export var orbit_speed    : float = 0.005  ## ドラッグ 1px あたりの回転量（rad）
@export var pitch_min_deg  : float = 10.0   ## 仰角の最小（地平より下に行かない）
@export var pitch_max_deg  : float = 85.0   ## 仰角の最大（真上）

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _target_node    : Node3D  = null
var _target_pos     : Vector3 = Vector3.ZERO
var _zoom_distance  : float   = 1800.0
var _yaw            : float   = 0.0    ## 水平回転（rad）
var _pitch          : float   = deg_to_rad(35.0)  ## 仰角（rad）
var _dragging       : bool    = false
var _drag_start     : Vector2 = Vector2.ZERO

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	pass

func _process(delta: float) -> void:
	if _target_node != null and is_instance_valid(_target_node):
		_target_pos = _target_node.global_position

	var desired: Vector3 = _target_pos + _orbit_offset()
	global_position = global_position.lerp(desired, follow_speed * delta)
	look_at(_target_pos, Vector3.UP)

func _unhandled_input(event: InputEvent) -> void:
	# ── ホイールズーム ────────────────────────────────────────────────────────
	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			if mb.button_index == MOUSE_BUTTON_WHEEL_UP:
				_zoom_distance = maxf(_zoom_distance - zoom_step, zoom_min)
				get_viewport().set_input_as_handled()
			elif mb.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				_zoom_distance = minf(_zoom_distance + zoom_step, zoom_max)
				get_viewport().set_input_as_handled()
			elif mb.button_index == MOUSE_BUTTON_LEFT:
				_dragging   = true
				_drag_start = mb.position
				get_viewport().set_input_as_handled()
		else:
			if mb.button_index == MOUSE_BUTTON_LEFT:
				_dragging = false

	# ── 左ドラッグで軌道回転 ─────────────────────────────────────────────────
	elif event is InputEventMouseMotion and _dragging:
		var mm: InputEventMouseMotion = event as InputEventMouseMotion
		_yaw   -= mm.relative.x * orbit_speed
		_pitch  = clampf(_pitch - mm.relative.y * orbit_speed,
		                 deg_to_rad(pitch_min_deg),
		                 deg_to_rad(pitch_max_deg))
		get_viewport().set_input_as_handled()

# ── 公開 API ──────────────────────────────────────────────────────────────────

func set_target(node: Node3D) -> void:
	_target_node = node
	if node != null:
		_target_pos     = node.global_position
		global_position = _target_pos + _orbit_offset()

## ドラッグ中かどうかを返す（main.gd がクリック判定に使う）
func is_dragging() -> bool:
	return _dragging

# ── 内部 ─────────────────────────────────────────────────────────────────────

func _orbit_offset() -> Vector3:
	## 球座標 → 直交座標
	var x: float = _zoom_distance * cos(_pitch) * sin(_yaw)
	var y: float = _zoom_distance * sin(_pitch)
	var z: float = _zoom_distance * cos(_pitch) * cos(_yaw)
	return Vector3(x, y, z)
