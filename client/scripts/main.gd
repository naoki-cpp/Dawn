## main.gd
##
## メインシーンのルートスクリプト。
## Connection からのイベントを受け取り、Ship ノードの生成・更新・削除を行う。

extends Node

# ── ノード参照 ────────────────────────────────────────────────────────────────

@onready var _connection : Node    = $Connection
@onready var _ships_root : Node3D  = $World/Ships
@onready var _stats_label: Label   = $HUD/StatsLabel

# ── 定数 ─────────────────────────────────────────────────────────────────────

const SHIP_SCENE := preload("res://scenes/ship.tscn")

# ── 内部状態 ─────────────────────────────────────────────────────────────────

## ship_id(int) → ShipController ノード
var _ships : Dictionary = {}

var _event_count : int = 0
var _current_tick: int = 0

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connection.event_received.connect(_on_event_received)
	_connection.connection_changed.connect(_on_connection_changed)
	_update_hud()

func _process(_delta: float) -> void:
	_update_hud()

# ── イベントハンドラ ──────────────────────────────────────────────────────────

func _on_event_received(payload: Dictionary) -> void:
	_event_count += 1
	var event_type: String = payload.get("type", "")

	match event_type:
		"ShipSpawned":
			_handle_ship_spawned(payload)
		"ShipMoved":
			_handle_ship_moved(payload)
		"ShipDespawned":
			_handle_ship_despawned(payload)
		_:
			push_warning("[Main] unknown event type: " + event_type)

func _on_connection_changed(connected: bool) -> void:
	if not connected:
		## 切断時は全 Ship を消去して再接続待ち
		_clear_all_ships()

# ── ドメインイベント処理 ──────────────────────────────────────────────────────

func _handle_ship_spawned(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0)
	if _ships.has(ship_id):
		return  ## 重複 spawn は無視

	var pos_dict : Dictionary = p.get("position", {})
	var pos := Vector3(
		float(pos_dict.get("x", 0.0)),
		float(pos_dict.get("y", 0.0)),
		float(pos_dict.get("z", 0.0)),
	)

	var ship : Node3D = SHIP_SCENE.instantiate()
	_ships_root.add_child(ship)
	ship.get_script().call("initialize", ship_id, pos)  # ShipController.initialize()
	ship.name = "Ship_%d" % ship_id
	_ships[ship_id] = ship

func _handle_ship_moved(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0)
	var ship = _ships.get(ship_id)
	if ship == null:
		return

	var pos_dict : Dictionary = p.get("to", {})
	var pos := Vector3(
		float(pos_dict.get("x", 0.0)),
		float(pos_dict.get("y", 0.0)),
		float(pos_dict.get("z", 0.0)),
	)
	ship.update_target(pos)

	var tick : int = p.get("tick", 0)
	if tick > _current_tick:
		_current_tick = tick

func _handle_ship_despawned(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0)
	var ship = _ships.get(ship_id)
	if ship != null:
		ship.queue_free()
		_ships.erase(ship_id)

# ── HUD 更新 ─────────────────────────────────────────────────────────────────

func _update_hud() -> void:
	_stats_label.text = (
		"Ships: %d\nTick: %d\nEvents: %d" % [_ships.size(), _current_tick, _event_count]
	)

# ── 内部ユーティリティ ────────────────────────────────────────────────────────

func _clear_all_ships() -> void:
	for ship in _ships.values():
		if is_instance_valid(ship):
			ship.queue_free()
	_ships.clear()
	_current_tick = 0
	_event_count  = 0
