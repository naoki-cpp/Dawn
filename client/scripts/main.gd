## main.gd
##
## メインシーンのルートスクリプト。
## Connection からのイベントを受け取り、Ship ノードの生成・更新・削除を行う。
##
## Cycle 2 追加:
##   - 最初に Spawn した Ship を「プレイヤー船」として扱う
##   - 左クリックで MoveCommand を送信（Y=0 平面へのレイキャスト）
##   - カメラをプレイヤー船に追従させる

extends Node

# ── ノード参照 ────────────────────────────────────────────────────────────────

@onready var _connection  : Node              = $Connection
@onready var _ships_root  : Node3D            = $World/Ships
@onready var _stats_label : Label             = $HUD/StatsLabel
@onready var _camera      : Camera3D          = $World/Camera3D
@onready var _move_marker : Node3D            = $World/MoveMarker

# ── 定数 ─────────────────────────────────────────────────────────────────────

const SHIP_SCENE := preload("res://scenes/ship.tscn")

# ── マテリアル（プレイヤー船ハイライト） ─────────────────────────────────────

var _player_material : StandardMaterial3D = null

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ships           : Dictionary = {}  ## ship_id(int) → Node3D
var _player_ship_id  : int        = -1  ## -1 = 未設定
var _event_count     : int        = 0
var _current_tick    : int        = 0

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connection.event_received.connect(_on_event_received)
	_connection.connection_changed.connect(_on_connection_changed)
	_build_player_material()
	_update_hud()

func _process(_delta: float) -> void:
	_update_hud()

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed and mb.button_index == MOUSE_BUTTON_LEFT:
			_on_left_click(mb.position)

# ── イベントハンドラ ──────────────────────────────────────────────────────────

func _on_event_received(payload: Dictionary) -> void:
	_event_count += 1
	var event_type: String = payload.get("type", "") as String
	match event_type:
		"ShipSpawned"   : _handle_ship_spawned(payload)
		"ShipMoved"     : _handle_ship_moved(payload)
		"ShipDespawned" : _handle_ship_despawned(payload)

func _on_connection_changed(connected: bool) -> void:
	if not connected:
		_clear_all_ships()

# ── 左クリック → MoveCommand ──────────────────────────────────────────────────

func _on_left_click(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	## カメラレイを Y=0 平面と交差させる
	var ray_origin : Vector3 = _camera.project_ray_origin(screen_pos)
	var ray_dir    : Vector3 = _camera.project_ray_normal(screen_pos)

	## Y=0 との交点: origin.y + t * dir.y = 0 → t = -origin.y / dir.y
	if absf(ray_dir.y) < 0.001:
		return  ## カメラが水平方向を向いている（ゼロ除算回避）

	var t         : float   = -ray_origin.y / ray_dir.y
	var world_pos : Vector3 = ray_origin + ray_dir * t

	## Godot 座標系 → サーバー座標系（WORLD_SCALE 逆変換 + Z 反転）
	var server_pos: Vector3 = Vector3(
		world_pos.x / 0.1,
		world_pos.y / 0.1,
		-world_pos.z / 0.1,
	)
	_connection.send_move_command(_player_ship_id, server_pos)

	## 移動先マーカーを表示
	_show_move_marker(world_pos)

# ── ドメインイベント処理 ──────────────────────────────────────────────────────

func _handle_ship_spawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if _ships.has(ship_id):
		return

	var pos_dict: Dictionary = p.get("position", {}) as Dictionary
	var pos := Vector3(
		(pos_dict.get("x", 0.0) as float),
		(pos_dict.get("y", 0.0) as float),
		(pos_dict.get("z", 0.0) as float),
	)

	var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
	_ships_root.add_child(ship)
	ship.call("initialize", ship_id, pos)
	ship.name = "Ship_%d" % ship_id
	_ships[ship_id] = ship

	## 最初の Spawn をプレイヤー船に指定
	if _player_ship_id < 0:
		_player_ship_id = ship_id
		_apply_player_material(ship)
		_camera.call("set_target", ship)
		print("[Main] player ship: %d" % ship_id)

func _handle_ship_moved(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var ship: Node3D = _ships[ship_id] as Node3D

	var pos_dict: Dictionary = p.get("to", {}) as Dictionary
	var pos := Vector3(
		(pos_dict.get("x", 0.0) as float),
		(pos_dict.get("y", 0.0) as float),
		(pos_dict.get("z", 0.0) as float),
	)
	ship.call("update_target", pos)

	var tick: int = p.get("tick", 0) as int
	if tick > _current_tick:
		_current_tick = tick

func _handle_ship_despawned(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var ship: Node3D = _ships[ship_id] as Node3D
	ship.queue_free()
	_ships.erase(ship_id)
	if ship_id == _player_ship_id:
		_player_ship_id = -1

# ── HUD ───────────────────────────────────────────────────────────────────────

func _update_hud() -> void:
	var connected: bool = _connection.is_connected_to_server()
	var status: String  = "ONLINE" if connected else "CONNECTING..."
	_stats_label.text = (
		"%s\nShips: %d\nTick: %d\nEvents: %d\n\n[LClick] Move"
		% [status, _ships.size(), _current_tick, _event_count]
	)

# ── 内部ユーティリティ ────────────────────────────────────────────────────────

func _clear_all_ships() -> void:
	for ship_node: Node3D in _ships.values():
		if is_instance_valid(ship_node):
			ship_node.queue_free()
	_ships.clear()
	_player_ship_id = -1
	_current_tick   = 0
	_event_count    = 0

func _build_player_material() -> void:
	_player_material = StandardMaterial3D.new()
	_player_material.albedo_color           = Color(1.0, 0.5, 0.1, 1)
	_player_material.metallic               = 0.9
	_player_material.roughness              = 0.2
	_player_material.emission_enabled       = true
	_player_material.emission               = Color(1.0, 0.3, 0.0, 1)
	_player_material.emission_energy_multiplier = 1.5

func _apply_player_material(ship: Node3D) -> void:
	var hull: MeshInstance3D = ship.get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.set_surface_override_material(0, _player_material)

func _show_move_marker(world_pos: Vector3) -> void:
	if _move_marker != null and is_instance_valid(_move_marker):
		_move_marker.global_position = world_pos
		_move_marker.visible = true
		## 少し経ったら消す
		get_tree().create_timer(1.5).timeout.connect(
			func() -> void:
				if is_instance_valid(_move_marker):
					_move_marker.visible = false
		)
