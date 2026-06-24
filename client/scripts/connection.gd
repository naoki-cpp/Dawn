## connection.gd
##
## ClientConnection の GDScript 側ラッパー。
## サーバー（Rust simulation）との WebSocket 通信を担当する。
##
## 設計 (ADR-0005, ADR-0007):
##   接続フロー:
##     1. 接続確立 → Hello を送信
##     2. Welcome を受信 → player_id / ship_id を保持、welcomed シグナル発行
##     3. InitialState を受信 → initial_state シグナル発行
##     4. 通常の DomainEvent ストリームを受信
##
## Phase 5: Hello/Welcome ハンドシェイク（ORIGIN シグナルを廃止）

extends Node

# ── シグナル ──────────────────────────────────────────────────────────────────

signal event_received(payload: Dictionary)
signal connection_changed(connected: bool)
## Welcome 受信時: player_id と ship_id を通知
signal welcomed(player_id: int, ship_id: int)
## On InitialState: notifies the full payload (ships + navigation map)
signal initial_state_received(state: Dictionary)
## On PlayerFitting (sent on connect and again after every Fit/Unfit, ADR-0032):
## notifies the full payload (modules + inventory + slot_capacity).
signal player_fitting_received(payload: Dictionary)
## ModuleActivated 受信時
signal module_activated(ship_id: int, module_id: int, slot: String)
## ModuleDeactivated 受信時
signal module_deactivated(ship_id: int, module_id: int, slot: String)

# ── 設定 ─────────────────────────────────────────────────────────────────────

const SERVER_URL         := "ws://127.0.0.1:7878"
const RECONNECT_INTERVAL := 2.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws              : WebSocketPeer = WebSocketPeer.new()
var _connected       : bool          = false
var _welcomed        : bool          = false   ## Welcome 受信済みか
var _reconnect_timer : float         = 0.0
var _buffer          : String        = ""

var player_id : int = -1
var ship_id   : int = -1

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connect_to_server()

func _process(delta: float) -> void:
	_ws.poll()
	var state: int = _ws.get_ready_state()

	if state == WebSocketPeer.STATE_OPEN:
		if not _connected:
			_connected = true
			_welcomed  = false
			print("[Connection] connected to ", SERVER_URL)
			connection_changed.emit(true)
			## 接続直後に Hello を送信する
			_send_hello()
		_receive_messages()

	elif state == WebSocketPeer.STATE_CLOSED:
		if _connected:
			_connected = false
			_welcomed  = false
			print("[Connection] disconnected, reconnecting in %.1fs" % RECONNECT_INTERVAL)
			connection_changed.emit(false)
		_reconnect_timer += delta
		if _reconnect_timer >= RECONNECT_INTERVAL:
			_reconnect_timer = 0.0
			_connect_to_server()

# ── 公開 API ──────────────────────────────────────────────────────────────────

func send_move_command(p_ship_id: int, target: Vector3) -> void:
	if not _welcomed:
		return
	var payload: Dictionary = {
		"type":    "MoveCommand",
		"ship_id": p_ship_id,
		"target":  { "x": target.x, "y": target.y, "z": target.z }
	}
	_ws.send_text(JSON.stringify(payload) + "\n")

func send_lock_on_command(p_ship_id: int, target_id: int) -> void:
	if not _welcomed:
		return
	var payload: Dictionary = {
		"type":      "LockOnCommand",
		"ship_id":   p_ship_id,
		"target_id": target_id,
	}
	_ws.send_text(JSON.stringify(payload) + "\n")

## Active モジュールをオンにする。
func send_activate_module(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "ActivateModuleCommand",
		"ship_id"  : p_ship_id,
		"module_id": p_module_id,
		"slot"     : p_slot,
	}) + "\n")

## Active モジュールをオフにする。
func send_deactivate_module(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "DeactivateModuleCommand",
		"ship_id"  : p_ship_id,
		"module_id": p_module_id,
		"slot"     : p_slot,
	}) + "\n")

## [S キー] 減速停止コマンド。サーバーが thrust を逆方向に掛けて速度ゼロまで減速する。
func send_stop_command(p_ship_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "StopCommand",
		"ship_id": p_ship_id,
	}) + "\n")

## ジャンプゲート経由の Sector 移動を要求する（ADR-0009）。
func send_jump_command(p_ship_id: int, p_gate_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "JumpCommand",
		"ship_id": p_ship_id,
		"gate_id": p_gate_id,
	}) + "\n")

## [A キー] アプローチ（半自動操船）。選択した船へ自動接近する（ADR-0015）。
func send_approach_command(p_ship_id: int, p_target_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "ApproachCommand",
		"ship_id"  : p_ship_id,
		"target_id": p_target_id,
	}) + "\n")

## [A キー] ジャンプゲートへアプローチ（半自動操船）。射程内まで自動接近する（ADR-0015）。
func send_approach_gate_command(p_ship_id: int, p_gate_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "ApproachCommand",
		"ship_id": p_ship_id,
		"gate_id": p_gate_id,
	}) + "\n")

## [W key] Warp (short-range Fold) to a Jump Gate (ADR-0022/ADR-0025).
func send_warp_command(p_ship_id: int, p_gate_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "WarpCommand",
		"ship_id": p_ship_id,
		"target" : { "Gate": p_gate_id },
	}) + "\n")

## [W key] Warp (short-range Fold) to a celestial body (ADR-0025).
func send_warp_to_body_command(p_ship_id: int, p_body_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "WarpCommand",
		"ship_id": p_ship_id,
		"target" : { "Body": p_body_id },
	}) + "\n")

## [O key] Orbit a selected ship at its weapon range (server-side default, ADR-0031).
func send_orbit_command(p_ship_id: int, p_target_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "OrbitCommand",
		"ship_id"  : p_ship_id,
		"target_id": p_target_id,
	}) + "\n")

## [O key] Orbit a selected Jump Gate at its weapon range (server-side default, ADR-0031).
func send_orbit_gate_command(p_ship_id: int, p_gate_id: int) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"   : "OrbitCommand",
		"ship_id": p_ship_id,
		"gate_id": p_gate_id,
	}) + "\n")

## [K key] Hold at least p_range_m metres from a selected ship; p_range_m <= 0
## falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_command(p_ship_id: int, p_target_id: int, p_range_m: float = -1.0) -> void:
	if not _welcomed:
		return
	var payload: Dictionary = {
		"type"     : "KeepAtRangeCommand",
		"ship_id"  : p_ship_id,
		"target_id": p_target_id,
	}
	if p_range_m > 0.0:
		payload["range"] = p_range_m
	_ws.send_text(JSON.stringify(payload) + "\n")

## [K key] Hold at least p_range_m metres from a selected Jump Gate; p_range_m
## <= 0 falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_gate_command(p_ship_id: int, p_gate_id: int, p_range_m: float = -1.0) -> void:
	if not _welcomed:
		return
	var payload: Dictionary = {
		"type"   : "KeepAtRangeCommand",
		"ship_id": p_ship_id,
		"gate_id": p_gate_id,
	}
	if p_range_m > 0.0:
		payload["range"] = p_range_m
	_ws.send_text(JSON.stringify(payload) + "\n")

## [Inventory panel] Move a module from inventory into a fitting slot (ADR-0032).
func send_fit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "FitModuleCommand",
		"ship_id"  : p_ship_id,
		"module_id": p_module_id,
		"slot"     : p_slot,
	}) + "\n")

## [Inventory panel] Move a fitted module back into inventory (ADR-0032).
func send_unfit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	if not _welcomed:
		return
	_ws.send_text(JSON.stringify({
		"type"     : "UnfitModuleCommand",
		"ship_id"  : p_ship_id,
		"module_id": p_module_id,
		"slot"     : p_slot,
	}) + "\n")

func is_connected_to_server() -> bool:
	return _connected and _welcomed

# ── 内部処理 ──────────────────────────────────────────────────────────────────

func _send_hello() -> void:
	_ws.send_text("{\"type\":\"Hello\"}\n")
	print("[Connection] Hello sent")

func _connect_to_server() -> void:
	print("[Connection] connecting to ", SERVER_URL, " ...")
	_ws = WebSocketPeer.new()
	var err: int = _ws.connect_to_url(SERVER_URL)
	if err != OK:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

func _receive_messages() -> void:
	while _ws.get_available_packet_count() > 0:
		var raw: String = _ws.get_packet().get_string_from_utf8()
		_buffer += raw
		_flush_buffer()

func _flush_buffer() -> void:
	while "\n" in _buffer:
		var idx : int    = _buffer.find("\n")
		var line: String = _buffer.left(idx).strip_edges()
		_buffer = _buffer.substr(idx + 1)
		if line.is_empty():
			continue
		var result: Variant = JSON.parse_string(line)
		if result == null:
			push_warning("[Connection] failed to parse JSON: " + line)
			continue
		var payload: Dictionary = result as Dictionary
		_handle_message(payload)

func _handle_message(payload: Dictionary) -> void:
	var msg_type: String = payload.get("type", "") as String
	match msg_type:
		"Welcome":
			player_id = payload.get("player_id", -1) as int
			ship_id   = payload.get("ship_id",   -1) as int
			_welcomed = true
			print("[Connection] Welcome: player_id=%d ship_id=%d" % [player_id, ship_id])
			welcomed.emit(player_id, ship_id)
		"InitialState":
			var ships: Array = payload.get("ships", []) as Array
			print("[Connection] InitialState: %d ships" % ships.size())
			initial_state_received.emit(payload)
		"PlayerFitting":
			var modules: Array = payload.get("modules", []) as Array
			print("[Connection] PlayerFitting: %d modules" % modules.size())
			player_fitting_received.emit(payload)
		"ModuleActivated":
			var sid: int    = payload.get("ship_id",   0)  as int
			var mid: int    = payload.get("module_id", 0)  as int
			var slt: String = payload.get("slot",      "") as String
			module_activated.emit(sid, mid, slt)
		"ModuleDeactivated":
			var sid: int    = payload.get("ship_id",   0)  as int
			var mid: int    = payload.get("module_id", 0)  as int
			var slt: String = payload.get("slot",      "") as String
			module_deactivated.emit(sid, mid, slt)
		_:
			event_received.emit(payload)
