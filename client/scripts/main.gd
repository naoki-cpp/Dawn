## main.gd
##
## メインシーンのルートスクリプト。
##
## Cycle 2:
##   - 左ドラッグ中はカメラ回転（クリックと区別）
##   - 左ダブルクリック → MoveCommand（加速度ベクトルを設定）
##   - 最初の Ship をプレイヤー船に指定 → set_player_ship シグナルをサーバーへ送信
##   - プレイヤー船はオレンジ色で表示

extends Node

# ── ノード参照 ────────────────────────────────────────────────────────────────

@onready var _connection  : Node     = $Connection
@onready var _ships_root  : Node3D   = $World/Ships
@onready var _stats_label : Label    = $HUD/StatsLabel
@onready var _camera      : Camera3D = $World/Camera3D

# ── 定数 ─────────────────────────────────────────────────────────────────────

const SHIP_SCENE  := preload("res://scenes/ship.tscn")
const WORLD_SCALE : float = 0.1   ## サーバー座標 ↔ Godot 座標の変換係数

# ── マテリアル ────────────────────────────────────────────────────────────────

var _player_material : StandardMaterial3D = null

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ships           : Dictionary = {}
var _player_ship_id  : int        = -1
var _event_count     : int        = 0
var _current_tick    : int        = 0
var _player_hp       : float      = -1.0   ## -1 = 未取得
var _player_max_hp   : float      = 1000.0 ## ShipStatsComp::PLAYER.max_hp
var _player_lock_target : int     = -1     ## 現在プレイヤーがロック中/ロック済みのターゲット

## ダブルクリック検出用
var _last_click_time  : float  = -1.0
var _last_click_pos   : Vector2 = Vector2.ZERO
const DOUBLE_CLICK_SEC: float  = 0.4   ## この秒数以内の2回クリックをダブルクリックと判定
const DOUBLE_CLICK_PX : float  = 10.0  ## この画素以内

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connection.event_received.connect(_on_event_received)
	_connection.connection_changed.connect(_on_connection_changed)
	_connection.welcomed.connect(_on_welcomed)
	_connection.initial_state_received.connect(_on_initial_state)
	_build_player_material()
	_setup_space_environment()
	_update_hud()

func _process(_delta: float) -> void:
	_update_hud()

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mb: InputEventMouseButton = event as InputEventMouseButton
		if mb.pressed:
			match mb.button_index:
				MOUSE_BUTTON_LEFT:
					## ダブルクリック検出（カメラドラッグ中は無視）
					_check_double_click(mb.position)
				MOUSE_BUTTON_RIGHT:
					## 右クリック → ロックオン対象を選択
					_try_lock_on(mb.position)

# ── ダブルクリック判定 ────────────────────────────────────────────────────────

func _check_double_click(pos: Vector2) -> void:
	var now: float = Time.get_ticks_msec() / 1000.0
	var dt : float = now - _last_click_time
	var dp : float = pos.distance_to(_last_click_pos)

	if dt < DOUBLE_CLICK_SEC and dp < DOUBLE_CLICK_PX:
		## カメラドラッグ中のダブルクリックは無視
		var cam_dragging: bool = (_camera as Node).call("is_dragging") as bool
		if not cam_dragging:
			_on_double_click(pos)
		_last_click_time = -1.0  ## リセット（3連打を2回目のダブルクリックにしない）
	else:
		_last_click_time = now
		_last_click_pos  = pos

# ── 右クリック → LockOnCommand ───────────────────────────────────────────────

func _try_lock_on(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	## レイキャストで画面上の点に対応する 3D 位置を取得
	var from: Vector3 = _camera.project_ray_origin(screen_pos)
	var dir : Vector3 = _camera.project_ray_normal(screen_pos)
	var to  : Vector3 = from + dir * 100_000.0

	## Ships 配下の全 Ship との交差判定
	var closest_id    : int   = -1
	var closest_dist  : float = 1e9

	for ship_id: int in _ships:
		var ship: Node3D = _ships[ship_id] as Node3D
		if ship_id == _player_ship_id:
			continue  # 自分自身はロック不可

		## 点とレイの最近傍距離でヒット判定（半径 500 Godot units）
		var p  : Vector3 = ship.global_position
		var t  : float   = (p - from).dot(dir)
		var closest_pt: Vector3 = from + dir * t
		var dist: float = p.distance_to(closest_pt)
		if dist < 500.0 and t > 0.0 and dist < closest_dist:
			closest_dist = dist
			closest_id   = ship_id

	if closest_id >= 0:
		## 前のロック対象をクリア
		if _player_lock_target >= 0 and _ships.has(_player_lock_target):
			(_ships[_player_lock_target] as Node3D).call("set_lock_state", "none")
		_player_lock_target = closest_id
		_connection.send_lock_on_command(_player_ship_id, closest_id)
		## ロック中（Locking）状態をセット + フラッシュ
		if _ships.has(closest_id):
			(_ships[closest_id] as Node3D).call("set_lock_state", "locking")
			(_ships[closest_id] as Node3D).call("flash_lock_indicator")

# ── ダブルクリック → MoveCommand ──────────────────────────────────────────────

func _on_double_click(screen_pos: Vector2) -> void:
	if _player_ship_id < 0:
		return

	## カメラレイの方向 = そのまま推力方向として使う（3D対応）
	var ray_dir: Vector3 = _camera.project_ray_normal(screen_pos)

	## Godot 座標系 → サーバー座標系の方向変換（Z反転のみ。スケールは正規化されるので不要）
	## Godot: (x, y, -z) = server (x, y, z) → 方向の場合: (dx, dy, -dz)
	var server_dir: Vector3 = Vector3(ray_dir.x, ray_dir.y, -ray_dir.z)

	## プレイヤー船のサーバー座標を推定（Godot 上の lerp 済み位置から逆算）
	var ship_godot_pos: Vector3 = Vector3.ZERO
	if _ships.has(_player_ship_id):
		ship_godot_pos = (_ships[_player_ship_id] as Node3D).global_position
	var ship_server_pos: Vector3 = Vector3(
		ship_godot_pos.x / WORLD_SCALE,
		ship_godot_pos.y / WORLD_SCALE,
		-ship_godot_pos.z / WORLD_SCALE,
	)

	## 目標を十分遠い点に設定 → サーバーは normalize(target - ship) ≈ server_dir として扱う
	var target: Vector3 = ship_server_pos + server_dir * 1_000_000.0
	_connection.send_move_command(_player_ship_id, target)

	## 推力矢印をプレイヤー船に表示（ray_dir は Godot 座標系のまま渡す）
	if _ships.has(_player_ship_id):
		(_ships[_player_ship_id] as Node3D).call("set_thrust_direction", ray_dir)

# ── イベントハンドラ ──────────────────────────────────────────────────────────

func _on_event_received(payload: Dictionary) -> void:
	_event_count += 1
	var event_type: String = payload.get("type", "") as String
	match event_type:
		"ShipSpawned"      : _handle_ship_spawned(payload)
		"VelocityChanged"  : _handle_velocity_changed(payload)
		"ShipDespawned"    : _handle_ship_despawned(payload)
		"DamageTaken"   : _handle_damage_taken(payload)
		"ShipDestroyed" : _handle_ship_destroyed(payload)
		"TargetLocked"  : _handle_target_locked(payload)
		"LockLost"      : _handle_lock_lost(payload)

func _on_connection_changed(connected: bool) -> void:
	if not connected:
		_clear_all_ships()

## Welcome 受信: player_id / ship_id を記録するだけ。
## 船ノードの生成は直後の InitialState で行う。
func _on_welcomed(_p_player_id: int, _p_ship_id: int) -> void:
	pass  ## connection.gd の ship_id / player_id プロパティに値が入っている

## InitialState 受信: 全船ノードをここで一括 spawn する。
## Phase 5 では ShipSpawned イベントは送信されないためこちらが初期化を担う。
func _on_initial_state(ships: Array) -> void:
	_clear_all_ships()  ## 再接続時に備えてリセット

	for ship_data: Variant in ships:
		var d        : Dictionary = ship_data as Dictionary
		var sid      : int        = d.get("ship_id",   0)   as int
		var pos_dict : Dictionary = d.get("position",  {})  as Dictionary
		var max_hp   : float      = d.get("max_hp",    1.0) as float
		var cur_hp   : float      = d.get("current_hp",1.0) as float
		var pos := Vector3(
			(pos_dict.get("x", 0.0) as float),
			(pos_dict.get("y", 0.0) as float),
			(pos_dict.get("z", 0.0) as float),
		)

		## 船ノードを生成
		var ship: Node3D = SHIP_SCENE.instantiate() as Node3D
		_ships_root.add_child(ship)
		ship.call("initialize", sid, pos)
		ship.name = "Ship_%d" % sid
		_ships[sid] = ship

		## 自分の船かどうか確認
		if sid == _connection.ship_id and _player_ship_id < 0:
			_player_max_hp = max_hp
			_player_hp     = cur_hp
			_set_as_player_ship(sid, ship)

func _set_as_player_ship(p_ship_id: int, ship: Node3D) -> void:
	_player_ship_id = p_ship_id
	_player_hp      = _player_max_hp
	_apply_player_material(ship)
	ship.call("set_as_player")
	_camera.call("set_target", ship)

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

	## Welcome で通知されたプレイヤー船 ID と一致すれば自分の船として設定
	if ship_id == _connection.ship_id and _player_ship_id < 0:
		_set_as_player_ship(ship_id, ship)

func _handle_velocity_changed(p: Dictionary) -> void:
	var ship_id : int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return

	var vel_dict: Dictionary = p.get("velocity", {}) as Dictionary
	## サーバー座標系 → Godot 座標系（Z 反転）
	var server_vel := Vector3(
		(vel_dict.get("dx", 0.0) as float),
		(vel_dict.get("dy", 0.0) as float),
		(vel_dict.get("dz", 0.0) as float),
	)
	(_ships[ship_id] as Node3D).call("set_velocity", server_vel)

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

func _handle_damage_taken(p: Dictionary) -> void:
	var ship_id   : int   = p.get("ship_id",    0)   as int
	var current_hp: float = p.get("current_hp", 0.0) as float
	if ship_id == _player_ship_id:
		_player_hp = current_hp
		## プレイヤーがダメージを受けたら赤フラッシュ
		if _ships.has(ship_id):
			(_ships[ship_id] as Node3D).call("flash_damage")

func _handle_ship_destroyed(p: Dictionary) -> void:
	var ship_id: int = p.get("ship_id", 0) as int
	if not _ships.has(ship_id):
		return
	var ship: Node3D = _ships[ship_id] as Node3D
	_ships.erase(ship_id)
	## 破壊エフェクトを再生（queue_free は play_destroy_effect 内で行う）
	ship.call("play_destroy_effect")
	if ship_id == _player_ship_id:
		_player_ship_id     = -1
		_player_hp          = 0.0
		_player_lock_target = -1
	## 破壊されたターゲットをロック中だった場合はクリア
	if ship_id == _player_lock_target:
		_player_lock_target = -1

func _handle_target_locked(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	## プレイヤーがロック完了した場合
	if locker_id == _player_ship_id:
		_player_lock_target = target_id
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "locked")
	## 他の船からロックされた場合（視覚的に表示しない）

func _handle_lock_lost(p: Dictionary) -> void:
	var locker_id: int = p.get("locker_id", 0) as int
	var target_id: int = p.get("target_id", 0) as int
	if locker_id == _player_ship_id:
		if _ships.has(target_id):
			(_ships[target_id] as Node3D).call("set_lock_state", "none")
		if target_id == _player_lock_target:
			_player_lock_target = -1

# ── HUD ───────────────────────────────────────────────────────────────────────

func _update_hud() -> void:
	var status: String = "ONLINE" if _connection.is_connected_to_server() else "CONNECTING..."

	var speed_str: String = "-"
	if _player_ship_id >= 0 and _ships.has(_player_ship_id):
		var spd: float = (_ships[_player_ship_id] as Node3D).call("get_speed_server") as float
		speed_str = "%d u/tick" % int(spd)

	var hp_str: String
	if _player_ship_id < 0:
		hp_str = "DESTROYED"
	elif _player_hp < 0.0:
		hp_str = "%.0f / %.0f" % [_player_max_hp, _player_max_hp]
	else:
		hp_str = "%.0f / %.0f" % [_player_hp, _player_max_hp]

	var lock_str: String
	if _player_lock_target < 0:
		lock_str = "-"
	elif _ships.has(_player_lock_target):
		lock_str = "→ #%d" % _player_lock_target
	else:
		lock_str = "LOST"

	_stats_label.text = (
		"%s\nShips: %d\nTick: %d\nSpeed: %s\nHP: %s\nLock: %s\n\n[DoubleClick] Thrust\n[RightClick] Lock"
		% [status, _ships.size(), _current_tick, speed_str, hp_str, lock_str]
	)

# ── 内部ユーティリティ ────────────────────────────────────────────────────────

func _clear_all_ships() -> void:
	for ship_node: Node3D in _ships.values():
		if is_instance_valid(ship_node):
			ship_node.queue_free()
	_ships.clear()
	_player_ship_id     = -1
	_player_hp          = -1.0
	_player_lock_target = -1
	_current_tick       = 0
	_event_count        = 0

func _setup_space_environment() -> void:
	## 宇宙スカイシェーダーを手続き的に構築する。
	## WorldEnvironment ノードを動的生成するため .tscn の変更が不要。
	var shader := load("res://shaders/space_sky.gdshader") as Shader
	if shader == null:
		push_warning("[Main] space_sky.gdshader が見つかりません")
		return

	var sky_mat := ShaderMaterial.new()
	sky_mat.shader = shader

	var sky := Sky.new()
	sky.sky_material      = sky_mat
	sky.process_mode      = Sky.PROCESS_MODE_REALTIME
	sky.radiance_size     = Sky.RADIANCE_SIZE_256

	var env := Environment.new()
	env.background_mode   = Environment.BG_SKY
	env.sky               = sky
	env.ambient_light_source  = Environment.AMBIENT_SOURCE_SKY
	env.ambient_light_energy  = 0.05  ## 宇宙は暗い
	env.tonemap_mode          = Environment.TONE_MAPPER_FILMIC

	var world_env          := WorldEnvironment.new()
	world_env.environment   = env
	add_child(world_env)

func _build_player_material() -> void:
	_player_material = StandardMaterial3D.new()
	_player_material.albedo_color               = Color(1.0, 0.5, 0.1, 1)
	_player_material.metallic                   = 0.9
	_player_material.roughness                  = 0.2
	_player_material.emission_enabled           = true
	_player_material.emission                   = Color(1.0, 0.3, 0.0, 1)
	_player_material.emission_energy_multiplier = 1.5

func _apply_player_material(ship: Node3D) -> void:
	var hull: MeshInstance3D = ship.get_node_or_null("Hull") as MeshInstance3D
	if hull != null:
		hull.set_surface_override_material(0, _player_material)
