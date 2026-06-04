## camera_controller.gd
##
## プレイヤー船を追従するストラテジックカメラ。
##
## 操作:
##   左ボタンドラッグ : カメラを軌道回転（ターゲット中心）
##   マウスホイール   : ズームイン / ズームアウト
##
## 設計:
##   左ボタン押下は消費しない → main.gd がダブルクリックを検出できる。
##   マウスが DRAG_THRESHOLD px 以上動いた時点でドラッグ開始と判定し、
##   それ以降のモーションイベントのみ消費する。

extends Camera3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

@export var follow_speed   : float = 5.0
@export var zoom_min       : float = 300.0
@export var zoom_max       : float = 8000.0
@export var zoom_step      : float = 250.0
@export var orbit_speed    : float = 0.005
@export var pitch_min_deg  : float = 10.0
@export var pitch_max_deg  : float = 85.0

## ドラッグと判定する最小移動量（ピクセル）
const DRAG_THRESHOLD : float = 6.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _target_node    : Node3D  = null
var _target_pos     : Vector3 = Vector3.ZERO
var _zoom_distance  : float   = 1800.0
var _yaw            : float   = 0.0
var _pitch          : float   = deg_to_rad(35.0)

var _btn_down       : bool    = false   ## 左ボタンが押されているか
var _drag_start     : Vector2 = Vector2.ZERO
var _dragging       : bool    = false   ## 実際のドラッグジェスチャーが始まったか

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	pass

func _process(delta: float) -> void:
	if _target_node != null and is_instance_valid(_target_node):
		_target_pos = _target_node.global_position

	var desired: Vector3 = _target_pos + _orbit_offset()
	global_position = global_position.lerp(desired, clampf(follow_speed * delta, 0.0, 1.0))
	look_at(_target_pos, Vector3.UP)

func _unhandled_input(event: InputEvent) -> void:
	# ── ホイールズーム（イベント消費する）────────────────────────────────────
	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			if mb.button_index == MOUSE_BUTTON_WHEEL_UP:
				_zoom_distance = maxf(_zoom_distance - zoom_step, zoom_min)
				get_viewport().set_input_as_handled()
				return
			elif mb.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				_zoom_distance = minf(_zoom_distance + zoom_step, zoom_max)
				get_viewport().set_input_as_handled()
				return

		# 左ボタン押下: ドラッグ開始の準備のみ。イベントは消費しない。
		if mb.button_index == MOUSE_BUTTON_LEFT:
			if mb.pressed:
				_btn_down   = true
				_dragging   = false
				_drag_start = mb.position
				# ★ set_input_as_handled() を呼ばない → main.gd がクリックを検出できる
			else:
				_btn_down = false
				_dragging = false

	# ── ドラッグ判定とカメラ回転 ─────────────────────────────────────────────
	elif event is InputEventMouseMotion and _btn_down:
		var mm: InputEventMouseMotion = event as InputEventMouseMotion

		# 閾値を超えたらドラッグ開始
		if not _dragging:
			if mm.position.distance_to(_drag_start) >= DRAG_THRESHOLD:
				_dragging = true

		if _dragging:
			_yaw   -= mm.relative.x * orbit_speed
			_pitch  = clampf(_pitch - mm.relative.y * orbit_speed,
			                 deg_to_rad(pitch_min_deg),
			                 deg_to_rad(pitch_max_deg))
			get_viewport().set_input_as_handled()

# ── 公開 API ──────────────────────────────────────────────────────────────────

func set_target(node: Node3D) -> void:
	_target_node = node
	if node != null:
		_target_pos    = node.global_position
		global_position = _target_pos + _orbit_offset()

## ドラッグ中かどうか（main.gd がダブルクリックと区別するために使う）
func is_dragging() -> bool:
	return _dragging

# ── 内部 ─────────────────────────────────────────────────────────────────────

func _orbit_offset() -> Vector3:
	var x: float = _zoom_distance * cos(_pitch) * sin(_yaw)
	var y: float = _zoom_distance * sin(_pitch)
	var z: float = _zoom_distance * cos(_pitch) * cos(_yaw)
	return Vector3(x, y, z)
