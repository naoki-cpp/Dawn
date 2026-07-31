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
## Owner-only authoritative normal-flight state for client prediction (ADR-0043).
signal motion_correction_received(payload: Dictionary)
signal connection_changed(connected: bool)
## Welcome 受信時: player_id と ship_id を通知
signal welcomed(player_id: int, ship_id: int)
## On InitialState: notifies the full payload (ships + navigation map)
signal initial_state_received(state: Dictionary)
## On PlayerLoadout (sent on connect and again after every Fit/Unfit,
## ADR-0032): carries the raw postcard bytes (ADR-0042), not a parsed
## Dictionary. `PlayerLoadout.apply_wire_bytes` (dawn-client-gdext) decodes
## them directly into typed Rust state, with no lossy Dictionary/JSON
## round-trip in between.
signal player_fitting_received(bytes: PackedByteArray)
## ModuleActivated 受信時
signal module_activated(ship_id: int, module_id: int, slot: String)
## ModuleDeactivated 受信時。reason は "cap" | "range" | ""（""=プレイヤー起因、ADR-0035）。
signal module_deactivated(ship_id: int, module_id: int, slot: String, reason: String)
## Current Market balance and bounded open-order snapshot.
signal market_snapshot_received(snapshot: Dictionary)

# ── 設定 ─────────────────────────────────────────────────────────────────────

const SERVER_URL         := "ws://127.0.0.1:7878"
const RECONNECT_INTERVAL := 2.0
## Keep retry latency short without writing one log record per attempt while
## the server is unavailable. Godot has no project-level log rotation setting,
## so the client must bound its own reconnect diagnostics.
const RECONNECT_LOG_INTERVAL := 30.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws              : WebSocketPeer = WebSocketPeer.new()
var _connected       : bool          = false
var _welcomed        : bool          = false   ## Welcome 受信済みか
var _reconnect_timer : float         = 0.0
var _reconnect_log_elapsed : float  = RECONNECT_LOG_INTERVAL
var _reconnect_attempts : int       = 0
var _server_url      : String        = SERVER_URL
## ClientCommand/ServerMessageDecoder are GDExtension classes (dawn-wire/
## dawn-client-gdext, ADR-0041/ADR-0042) -- globally registered, no preload
## needed. Their methods take &self (matching every other GDExtension class
## in this project, e.g. PlayerLoadout), so callers need an instance rather
## than calling the class name directly.
var _cmd             : ClientCommand         = ClientCommand.new()
var _decoder         : ServerMessageDecoder  = ServerMessageDecoder.new()

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
			_reconnect_attempts = 0
			_reconnect_log_elapsed = 0.0
			print("[Connection] connected to ", _server_url)
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
		_reconnect_log_elapsed += delta
		if _reconnect_timer >= RECONNECT_INTERVAL:
			_reconnect_timer = 0.0
			_connect_to_server()

# ── 公開 API ──────────────────────────────────────────────────────────────────

## ADR-0037: flight/steering/module/Undock commands carry no ship_id — the
## server always resolves them against the caller's active ship, so there is
## no wire-representable way to name a ship the player isn't currently
## flying. Station inventory-management commands (Fit/Unfit/Dock/
## BuildPackagedShip/DisassembleShip) still carry an explicit ship_id.
##
## ADR-0041/ADR-0042: every send_* function below builds its wire message
## via `ClientCommand` (a `dawn-wire`-backed GDExtension class, globally
## registered like `PlayerLoadout`/`ModuleRow`/`ItemRow` -- no preload
## needed), instead of hand-building a matching Dictionary + JSON.stringify.
## Commands with sentinel/tagged-target semantics (ADR-0031/ADR-0035)
## call a dedicated `_cmd.*_command()` method; everything else goes through
## `_cmd.build(type_tag, fields)`, which validates the field Dictionary by
## deserializing it into `ClientCommandWire` itself. Every method returns
## postcard-encoded bytes already wrapped in the `ClientMessage::Command`
## envelope (ADR-0042); `_send_bytes` only applies the welcomed guard.
func send_move_command(target: Vector3) -> void:
	_send_bytes(_cmd.move_command(target.x, target.y, target.z))

func send_lock_on_command(target_id: int) -> void:
	_send_bytes(_cmd.build("LockOnCommand", {"target_id": target_id}))

## Active モジュールをオンにする。p_target_ship_id は Weapon/Tackle など
## ターゲットを要求する種別のときだけ指定する（-1 = 指定なし、ADR-0035）。
func send_activate_module(p_module_id: int, p_slot: String, p_target_ship_id: int = -1) -> void:
	_send_bytes(_cmd.activate_module_command(p_module_id, p_slot, p_target_ship_id))

## Active モジュールをオフにする。
func send_deactivate_module(p_module_id: int, p_slot: String) -> void:
	_send_bytes(_cmd.build("DeactivateModuleCommand", {"module_id": p_module_id, "slot": p_slot}))

## [S キー] 減速停止コマンド。サーバーが thrust を逆方向に掛けて速度ゼロまで減速する。
func send_stop_command() -> void:
	_send_bytes(_cmd.build("StopCommand", {}))

## ジャンプゲート経由の Sector 移動を要求する（ADR-0009）。
func send_jump_command(p_gate_id: int) -> void:
	_send_bytes(_cmd.build("JumpCommand", {"gate_id": p_gate_id}))

## [A キー] アプローチ（半自動操船）。選択した船へ自動接近する（ADR-0015）。
func send_approach_command(p_target_id: int) -> void:
	_send_bytes(_cmd.approach_command(p_target_id))

## [A キー] ジャンプゲートへアプローチ（半自動操船）。射程内まで自動接近する（ADR-0015）。
func send_approach_gate_command(p_gate_id: int) -> void:
	_send_bytes(_cmd.approach_gate_command(p_gate_id))

## [W key] Warp (short-range Fold) to a Jump Gate (ADR-0022/ADR-0025).
func send_warp_command(p_gate_id: int) -> void:
	_send_bytes(_cmd.warp_command(p_gate_id))

## [W key] Warp (short-range Fold) to a celestial body (ADR-0025).
func send_warp_to_body_command(p_body_id: int) -> void:
	_send_bytes(_cmd.warp_to_body_command(p_body_id))

## [O key] Orbit a selected ship at its weapon range (server-side default, ADR-0031).
func send_orbit_command(p_target_id: int) -> void:
	_send_bytes(_cmd.orbit_command(p_target_id, -1.0))

## [O key] Orbit a selected Jump Gate at its weapon range (server-side default, ADR-0031).
func send_orbit_gate_command(p_gate_id: int) -> void:
	_send_bytes(_cmd.orbit_gate_command(p_gate_id, -1.0))

## [K key] Hold at least p_range_m metres from a selected ship; p_range_m <= 0
## falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_command(p_target_id: int, p_range_m: float = -1.0) -> void:
	_send_bytes(_cmd.keep_at_range_command(p_target_id, p_range_m))

## [K key] Hold at least p_range_m metres from a selected Jump Gate; p_range_m
## <= 0 falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_gate_command(p_gate_id: int, p_range_m: float = -1.0) -> void:
	_send_bytes(_cmd.keep_at_range_gate_command(p_gate_id, p_range_m))

## [Inventory panel] Move a module from inventory into a fitting slot (ADR-0032).
func send_fit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	_send_bytes(_cmd.build("FitModuleCommand", {
		"ship_id": p_ship_id, "module_id": p_module_id, "slot": p_slot}))

## [Inventory panel] Move a fitted module back into inventory (ADR-0032).
func send_unfit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	_send_bytes(_cmd.build("UnfitModuleCommand", {
		"ship_id": p_ship_id, "module_id": p_module_id, "slot": p_slot}))

## [Inventory panel] Reorder two fitted modules within the same slot kind
## (drag-and-drop reorder in the FITTED column). Persisted server-side since
## iteration order assigns weapon hotkey F-numbers -- see ADR-0032's
## 2026-07-08 amendment.
func send_reorder_fitted_module_command(
	p_ship_id: int, p_slot: String, p_from_index: int, p_to_index: int
) -> void:
	_send_bytes(_cmd.build("ReorderFittedModuleCommand", {
		"ship_id": p_ship_id, "slot": p_slot,
		"from_index": p_from_index, "to_index": p_to_index}))

func send_dock_command(p_station_id: int) -> void:
	_send_bytes(_cmd.build("DockCommand", {"station_id": p_station_id}))

func send_undock_command() -> void:
	_send_bytes(_cmd.build("UndockCommand", {}))

func send_build_packaged_ship_command(p_ship_id: int, p_station_id: int, p_ship_type_id: int) -> void:
	_send_bytes(_cmd.build("BuildPackagedShipCommand", {
		"ship_id": p_ship_id, "station_id": p_station_id, "ship_type_id": p_ship_type_id}))

func send_disassemble_ship_command(p_ship_id: int, p_station_id: int) -> void:
	_send_bytes(_cmd.build("DisassembleShipCommand", {
		"ship_id": p_ship_id, "station_id": p_station_id}))

## Convert a station-inventory Packaged Ship item into a new live docked ship
## (ADR-0034 9B, ADR-0037). No ship_id -- the ship doesn't exist yet.
func send_assemble_command(p_station_id: int, p_ship_type_id: int) -> void:
	_send_bytes(_cmd.build("AssembleCommand", {
		"station_id": p_station_id, "ship_type_id": p_ship_type_id}))

## Leave the active ship while docked, without disassembling it (ADR-0037).
func send_disembark_command() -> void:
	_send_bytes(_cmd.build("DisembarkCommand", {}))

## Make an owned, docked ship the caller's active ship (ADR-0037). This is
## how a player re-boards after Disembark, or switches between owned ships.
func send_select_active_ship_command(p_ship_id: int) -> void:
	_send_bytes(_cmd.build("SelectActiveShipCommand", {"ship_id": p_ship_id}))

## Move the entire stack of an item out of a docked ship's own cargo into
## the caller's station inventory (ADR-0034 9B). p_item_type is one of
## "Module", "PackagedShip", "ScrapMetal" (matches ItemRow.item_type);
## p_module_id/p_ship_type_id are only meaningful for the matching variant.
func send_transfer_to_station_command(
	p_ship_id: int,
	p_station_id: int,
	p_item_type: String,
	p_module_id: int = 0,
	p_ship_type_id: int = 0
) -> void:
	_send_bytes(_cmd.transfer_to_station_command(
		p_ship_id, p_station_id, p_item_type, p_module_id, p_ship_type_id))

## The reverse of send_transfer_to_station_command: move the entire stack of
## an item out of the caller's station inventory back into the docked ship's
## own cargo.
func send_transfer_from_station_command(
	p_ship_id: int,
	p_station_id: int,
	p_item_type: String,
	p_module_id: int = 0,
	p_ship_type_id: int = 0
) -> void:
	_send_bytes(_cmd.transfer_from_station_command(
		p_ship_id, p_station_id, p_item_type, p_module_id, p_ship_type_id))

## Market requests use a separate wire envelope from Sector commands
## (ADR-0034). The server answers each request with MarketSnapshot.
func send_market_refresh_command() -> void:
	_send_bytes(_cmd.market_build("RefreshMarketCommand", {}))

func send_market_place_order_command(
	p_ship_id: int,
	p_item_type: String,
	p_module_id: int,
	p_ship_type_id: int,
	p_side: String,
	p_price: int,
	p_quantity: int
) -> void:
	_send_bytes(_cmd.market_build("PlaceMarketOrderCommand", {
		"ship_id": p_ship_id,
		"item_type": p_item_type,
		"module_id": p_module_id,
		"ship_type_id": p_ship_type_id,
		"side": p_side,
		"price": p_price,
		"quantity": p_quantity,
	}))

func send_market_cancel_order_command(p_order_id: int) -> void:
	_send_bytes(_cmd.market_build("CancelMarketOrderCommand", {
		"order_id": p_order_id,
	}))

func is_connected_to_server() -> bool:
	return _connected and _welcomed

# ── 内部処理 ──────────────────────────────────────────────────────────────────

## welcomed ガードを一元化する send_* 系の共通ヘルパー。bytes は
## _cmd.*_command()/_cmd.build() が返す、すでに `ClientMessage::Command`
## envelope 済みの postcard バイト列（ADR-0042）。Hello（welcomed 前に送る
## 必要がある）はこのガードの対象外なので _send_hello は使わない。
func _send_bytes(bytes: PackedByteArray) -> void:
	if not _welcomed or bytes.is_empty():
		return
	_ws.send(bytes, WebSocketPeer.WRITE_MODE_BINARY)

func _send_hello() -> void:
	_ws.send(_cmd.hello_command(player_id, ship_id), WebSocketPeer.WRITE_MODE_BINARY)
	print("[Connection] Hello sent")

func _connect_to_server() -> void:
	_reconnect_attempts += 1
	var should_log_attempt := should_log_reconnect(
		_reconnect_attempts,
		_reconnect_log_elapsed,
		RECONNECT_LOG_INTERVAL)
	if should_log_attempt:
		_reconnect_log_elapsed = 0.0
		print("[Connection] connecting to %s ... (attempt %d)" % [
			_server_url,
			_reconnect_attempts])
	_ws = WebSocketPeer.new()
	var err: int = _ws.connect_to_url(_server_url)
	if err != OK and should_log_attempt:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

## Reconnect attempts remain frequent for responsiveness, but their diagnostics
## are emitted only on the first attempt and at the bounded interval thereafter.
static func should_log_reconnect(attempt: int, elapsed: float, interval: float) -> bool:
	return attempt <= 1 or elapsed >= interval

## One WebSocket frame always carries exactly one message (ADR-0042). Every
## server -> client message is the postcard `ServerMessage` binary envelope
## (ADR-0042 stages 1-2c, migration complete); there is no text-frame path
## to fall back to (issue #179). `ServerMessageDecoder` converts most binary
## variants (including `InitialState`, `AoiEnter`/`AoiLeave`, `PositionSnap`,
## `MotionCorrection`) into the same `{"type": ..., ...}` Dictionary shape the
## old JSON messages used, except `PlayerLoadout` (ADR-0042 2a), which it
## reduces to a bare `{"type": "PlayerLoadout"}` dispatch tag -- the raw
## bytes go straight to `PlayerLoadout.apply_wire_bytes` instead, bypassing
## the Dictionary entirely for precision (see `player_fitting_received`).
func _receive_messages() -> void:
	while _ws.get_available_packet_count() > 0:
		var packet: PackedByteArray = _ws.get_packet()
		var payload: Dictionary = _decoder.decode(packet)
		if payload.is_empty():
			push_warning("[Connection] failed to decode binary ServerMessage")
			continue
		_handle_message(payload, packet)

func _handle_message(payload: Dictionary, raw_bytes: PackedByteArray) -> void:
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
		"PlayerLoadout", "PlayerFitting":
			print("[Connection] PlayerLoadout received")
			player_fitting_received.emit(raw_bytes)
		"Redirect":
			_handle_redirect(payload)
		"ModuleActivated":
			var sid: int    = payload.get("ship_id",   0)  as int
			var mid: int    = payload.get("module_id", 0)  as int
			var slt: String = payload.get("slot",      "") as String
			module_activated.emit(sid, mid, slt)
		"ModuleDeactivated":
			var sid: int    = payload.get("ship_id",   0)  as int
			var mid: int    = payload.get("module_id", 0)  as int
			var slt: String = payload.get("slot",      "") as String
			var rsn: String = payload.get("reason",    "") as String
			module_deactivated.emit(sid, mid, slt, rsn)
		"MarketSnapshot":
			market_snapshot_received.emit(payload)
		"MotionCorrection":
			motion_correction_received.emit(payload)
		_:
			event_received.emit(payload)

func _handle_redirect(payload: Dictionary) -> void:
	var ws_addr: String = payload.get("ws_addr", "") as String
	if ws_addr.is_empty():
		push_warning("[Connection] Redirect without ws_addr")
		return
	player_id = payload.get("player_id", player_id) as int
	ship_id = payload.get("ship_id", ship_id) as int
	_server_url = _normalize_ws_url(ws_addr)
	_welcomed = false
	_connected = false
	_reconnect_timer = RECONNECT_INTERVAL
	print("[Connection] Redirect: reconnecting to %s as player_id=%d ship_id=%d" % [_server_url, player_id, ship_id])
	connection_changed.emit(false)
	_ws.close()

func _normalize_ws_url(addr: String) -> String:
	if addr.begins_with("ws://") or addr.begins_with("wss://"):
		return addr
	return "ws://" + addr
