extends "res://scripts/connection.gd"

## Controlled transport adapter. Tests exercise the real connection lifecycle,
## message callbacks, credential handling and dialog instead of a fake policy.
class Socket extends RefCounted:
	var state := WebSocketPeer.STATE_CONNECTING
	var handshake_headers := PackedStringArray()
	var heartbeat_interval := 0.0
	var sent: Array[PackedByteArray] = []

	func connect_to_url(_url: String) -> int:
		return OK

	func poll() -> void:
		pass

	func get_ready_state() -> int:
		return state

	func get_available_packet_count() -> int:
		return 0

	func send(bytes: PackedByteArray, _mode: int) -> int:
		sent.append(bytes)
		return OK

	func close() -> void:
		state = WebSocketPeer.STATE_CLOSED


var sockets: Array[Socket] = []


func _new_socket() -> Variant:
	var socket := Socket.new()
	sockets.append(socket)
	return socket


func open_socket() -> void:
	sockets[-1].state = WebSocketPeer.STATE_OPEN
	_process(0.0)


func close_socket() -> void:
	sockets[-1].close()
	_process(0.0)
