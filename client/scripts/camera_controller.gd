## camera_controller.gd
##
## プレイヤー船を追従するストラテジックカメラ。
##
## 操作:
##   マウスホイール : ズームイン / ズームアウト
##   中ボタンドラッグ: 水平方向パン
##   右ドラッグ     : 軌道回転（将来拡張用、現在は仰角固定）

extends Camera3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

## 追従オフセット（プレイヤー船の位置からの相対座標）
@export var follow_offset   : Vector3 = Vector3(0.0, 1200.0, 1800.0)
## 追従の滑らかさ（大きいほど即座に追従）
@export var follow_speed    : float   = 4.0
## ズーム最小距離
@export var zoom_min        : float   = 300.0
## ズーム最大距離
@export var zoom_max        : float   = 8000.0
## 1 ステップあたりのズーム量
@export var zoom_step       : float   = 200.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _target_node  : Node3D = null   ## 追従ターゲット（プレイヤー船）
var _target_pos   : Vector3 = Vector3.ZERO
var _zoom_distance: float  = 0.0    ## オフセットのスカラー倍率

func _ready() -> void:
	_zoom_distance = follow_offset.length()

func _process(delta: float) -> void:
	if _target_node != null and is_instance_valid(_target_node):
		_target_pos = _target_node.global_position

	var desired: Vector3 = _target_pos + _get_scaled_offset()
	global_position = global_position.lerp(desired, follow_speed * delta)
	look_at(_target_pos, Vector3.UP)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			if mb.button_index == MOUSE_BUTTON_WHEEL_UP:
				_zoom_distance = maxf(_zoom_distance - zoom_step, zoom_min)
			elif mb.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				_zoom_distance = minf(_zoom_distance + zoom_step, zoom_max)

# ── 公開 API ──────────────────────────────────────────────────────────────────

## 追従ターゲットを設定する。
func set_target(node: Node3D) -> void:
	_target_node = node
	if node != null:
		_target_pos = node.global_position
		global_position = _target_pos + _get_scaled_offset()

# ── 内部 ─────────────────────────────────────────────────────────────────────

func _get_scaled_offset() -> Vector3:
	var dir: Vector3 = follow_offset.normalized()
	return dir * _zoom_distance
