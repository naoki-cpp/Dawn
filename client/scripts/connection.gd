## connection.gd
##
## ClientConnection の GDScript 側ラッパー。
## サーバー（Rust simulation）との WebSocket 通信を担当する。
##
## 設計 (ADR-0005):
##   サーバー → クライアント : DomainEvent の JSON ストリーム（改行区切り）
##   クライアント → サーバー : Command の JSON 送信
##
## Phase 4: WebSocket over localhost:7878
## Phase 5: gRPC/QUIC に差し替え（このファイルのみ変更）

extends Node

# ── シグナル ──────────────────────────────────────────────────────────────────

## サーバーから DomainEvent を受け取ったときに発火する。
## payload: { "type": "ShipSpawned" | "ShipMoved" | "ShipDespawned", ... }
signal event_received(payload: Dictionary)

## 接続状態が変化したときに発火する。
signal connection_changed(connected: bool)

# ── 設定 ─────────────────────────────────────────────────────────────────────

## サーバーの WebSocket エンドポイント
const SERVER_URL := "ws://127.0.0.1:7878"

## 再接続間隔（秒）
const RECONNECT_INTERVAL := 2.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws       := WebSocketPeer.new()
var _connected := false
var _reconnect_timer := 0.0
var _buffer    := ""        ## 受信途中の改行区切りデータ

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connect_to_server()

func _process(delta: float) -> void:
	_ws.poll()

	var state := _ws.get_ready_state()

	match state:
		WebSocketPeer.STATE_OPEN:
			if not _connected:
				_connected = true
				print("[Connection] connected to ", SERVER_URL)
				connection_changed.emit(true)
			_receive_messages()

		WebSocketPeer.STATE_CLOSED:
			if _connected:
				_connected = false
				print("[Connection] disconnected, reconnecting in %.1fs" % RECONNECT_INTERVAL)
				connection_changed.emit(false)
			_reconnect_timer += delta
			if _reconnect_timer >= RECONNECT_INTERVAL:
				_reconnect_timer = 0.0
				_connect_to_server()

# ── 公開 API ──────────────────────────────────────────────────────────────────

## MoveCommand をサーバーへ送信する。
## ship_id: int（EntityId の u64 値）
## target: Vector3
func send_move_command(ship_id: int, target: Vector3) -> void:
	if not _connected:
		return
	var payload := {
		"type":   "MoveCommand",
		"ship_id": ship_id,
		"target": { "x": target.x, "y": target.y, "z": target.z }
	}
	_ws.send_text(JSON.stringify(payload) + "\n")

## 現在接続中かどうかを返す。
func is_connected_to_server() -> bool:
	return _connected

# ── 内部処理 ──────────────────────────────────────────────────────────────────

func _connect_to_server() -> void:
	print("[Connection] connecting to ", SERVER_URL, " ...")
	_ws = WebSocketPeer.new()
	var err := _ws.connect_to_url(SERVER_URL)
	if err != OK:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

func _receive_messages() -> void:
	while _ws.get_available_packet_count() > 0:
		var raw := _ws.get_packet().get_string_from_utf8()
		_buffer += raw
		_flush_buffer()

func _flush_buffer() -> void:
	## 改行区切りで JSON を切り出す（1 行 = 1 イベント）
	while "\n" in _buffer:
		var idx := _buffer.find("\n")
		var line := _buffer.left(idx).strip_edges()
		_buffer  = _buffer.substr(idx + 1)
		if line.is_empty():
			continue
		var result := JSON.parse_string(line)
		if result == null:
			push_warning("[Connection] failed to parse JSON: " + line)
			continue
		event_received.emit(result)
