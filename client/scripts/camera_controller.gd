## camera_controller.gd
##
## プレイヤー船を追従する3Dオービットカメラ。
##
## 操作:
##   左ボタンドラッグ : カメラを軌道回転（上下左右、全方向）
##   マウスホイール   : ズームイン / ズームアウト
##
## 回転方式: クォータニオン（ジンバルロックなし）
##   水平ドラッグ → ワールド Y 軸まわり yaw
##   垂直ドラッグ → カメラのローカル右軸まわり pitch
##   → 真上・真下・後ろからも見られる

extends Camera3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

@export var follow_speed   : float = 5.0
@export var zoom_min       : float = 200.0
@export var zoom_max       : float = 10000.0
@export var zoom_step      : float = 300.0
@export var orbit_speed    : float = 0.005  ## ドラッグ 1px あたりの回転量（rad）

const DRAG_THRESHOLD : float = 6.0  ## ドラッグ判定の最小移動量（px）

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _target_node   : Node3D    = null
var _target_pos    : Vector3   = Vector3.ZERO
var _zoom_distance : float     = 1800.0

## カメラの向きをクォータニオンで管理
## 初期値: 斜め上後ろから見る角度
var _orbit_quat : Quaternion = (
	Quaternion(Vector3.UP, 0.0) *
	Quaternion(Vector3.RIGHT, -deg_to_rad(35.0))
)

var _btn_down    : bool    = false
var _drag_start  : Vector2 = Vector2.ZERO
var _dragging    : bool    = false

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	pass

func _process(delta: float) -> void:
	if _target_node != null and is_instance_valid(_target_node):
		_target_pos = _target_node.global_position

	var desired : Vector3 = _target_pos + _orbit_offset()
	global_position = global_position.lerp(desired, clampf(follow_speed * delta, 0.0, 1.0))

	## look_at の up ベクトルをクォータニオンから導く → ジンバルロックしない
	var cam_up : Vector3 = (_orbit_quat * Vector3.UP).normalized()
	look_at(_target_pos, cam_up)

func _unhandled_input(event: InputEvent) -> void:
	# ── ホイールズーム ─────────────────────────────────────────────────────────
	if event is InputEventMouseButton:
		var mb : InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			if mb.button_index == MOUSE_BUTTON_WHEEL_UP:
				_zoom_distance = maxf(_zoom_distance - zoom_step, zoom_min)
				get_viewport().set_input_as_handled()
				return
			elif mb.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				_zoom_distance = minf(_zoom_distance + zoom_step, zoom_max)
				get_viewport().set_input_as_handled()
				return

		# 左ボタン押下: ドラッグ開始の準備のみ（消費しない）
		if mb.button_index == MOUSE_BUTTON_LEFT:
			if mb.pressed:
				_btn_down   = true
				_dragging   = false
				_drag_start = mb.position
			else:
				_btn_down = false
				_dragging = false

	# ── ドラッグで軌道回転 ─────────────────────────────────────────────────────
	elif event is InputEventMouseMotion and _btn_down:
		var mm : InputEventMouseMotion = event as InputEventMouseMotion

		if not _dragging:
			if mm.position.distance_to(_drag_start) >= DRAG_THRESHOLD:
				_dragging = true

		if _dragging:
			_apply_orbit(mm.relative.x, mm.relative.y)
			get_viewport().set_input_as_handled()

# ── 公開 API ──────────────────────────────────────────────────────────────────

func set_target(node: Node3D) -> void:
	_target_node = node
	if node != null:
		_target_pos     = node.global_position
		global_position = _target_pos + _orbit_offset()

## Called by main.gd when the floating origin rebases (ADR-0029): the whole world
## (including the follow target) shifted by `shift`, so shift the cached target
## too. main.gd already shifted our global_position, so the orbit offset and
## look_at stay continuous and the view doesn't swing for a frame.
func on_origin_rebased(shift: Vector3) -> void:
	_target_pos += shift

func is_dragging() -> bool:
	return _dragging

# ── 内部 ─────────────────────────────────────────────────────────────────────

## クォータニオン軌道回転を適用する。
## 水平: ワールド Y 軸まわり（宇宙感覚で水平に振れる）
## 垂直: カメラの現在のローカル右軸まわり（上下に回れる）
func _apply_orbit(dx: float, dy: float) -> void:
	# yaw: ワールドY軸まわり
	var yaw   : Quaternion = Quaternion(Vector3.UP, -dx * orbit_speed)

	# pitch: カメラのローカル右軸まわり（ワールド空間に変換済み）
	var cam_right : Vector3    = (_orbit_quat * Vector3.RIGHT).normalized()
	var pitch     : Quaternion = Quaternion(cam_right, -dy * orbit_speed)

	_orbit_quat = (pitch * yaw * _orbit_quat).normalized()

## クォータニオンからカメラ位置のオフセットを計算する。
## 基底: (0, 0, zoom) をクォータニオンで回転させた方向に配置。
func _orbit_offset() -> Vector3:
	return _orbit_quat * Vector3(0.0, 0.0, _zoom_distance)
