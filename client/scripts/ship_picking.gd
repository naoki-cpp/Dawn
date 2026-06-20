## ship_picking.gd
##
## Ray-vs-candidate picking math for ship/gate/body selection, extracted from
## main.gd (architecture-review-client.md C-1: main.gd was a god object;
## this is the first piece pulled out). Stateless static helpers -- every
## method takes the camera and candidate data it needs and returns the
## picked id, or -1. No @onready / scene-path dependencies, so this class
## (and ray_point_distance specifically) is unit-testable; see
## client/test/ship_picking_test.gd.
class_name ShipPicking
extends RefCounted

const PICK_RADIUS_SHIP: float = 500.0
const PICK_RADIUS_GATE: float = 300.0
const PICK_RADIUS_BODY_MIN: float = 400.0
const PICK_RADIUS_BODY_SCALE: float = 0.15


## Perpendicular distance from world point `p` to the ray (`from`, `dir`),
## packed as (dist, t) — `t` is the ray parameter at the closest approach
## (t <= 0 means `p` is behind the camera).
static func ray_point_distance(from: Vector3, dir: Vector3, p: Vector3) -> Vector2:
	var t: float = (p - from).dot(dir)
	return Vector2(p.distance_to(from + dir * t), t)


## Returns the ship_id whose node is closest to the click ray (within
## PICK_RADIUS_SHIP Godot units), excluding `exclude_id` (the player's own
## ship). -1 if nothing is hit.
static func pick_ship_at(camera: Camera3D, screen_pos: Vector2, ships: Dictionary, exclude_id: int) -> int:
	var from: Vector3 = camera.project_ray_origin(screen_pos)
	var dir : Vector3 = camera.project_ray_normal(screen_pos)
	var closest_id  : int   = -1
	var closest_dist: float = 1e9
	for ship_id: int in ships:
		if ship_id == exclude_id:
			continue
		var p : Vector3 = (ships[ship_id] as Node3D).global_position
		var dt: Vector2 = ray_point_distance(from, dir, p)
		if dt.x < PICK_RADIUS_SHIP and dt.y > 0.0 and dt.x < closest_dist:
			closest_dist = dt.x
			closest_id   = ship_id
	return closest_id


## Returns the gate_id of the Jump Gate (in the current system) whose marker
## is closest to the click ray, or -1. `to_godot_pos` converts a gate's
## server-space position into Godot world space (main.gd's
## _server_to_godot_pos) -- gates are large objects, so the pick radius is
## wider than for ships.
static func pick_gate_at(camera: Camera3D, screen_pos: Vector2, gates: Array, to_godot_pos: Callable) -> int:
	var from: Vector3 = camera.project_ray_origin(screen_pos)
	var dir : Vector3 = camera.project_ray_normal(screen_pos)
	var closest_id  : int   = -1
	var closest_dist: float = 1e9
	for gate: Variant in gates:
		var g: Dictionary = gate as Dictionary
		var p : Vector3 = to_godot_pos.call(g.get("position", Vector3.ZERO) as Vector3) as Vector3
		var dt: Vector2 = ray_point_distance(from, dir, p)
		if dt.x < PICK_RADIUS_GATE and dt.y > 0.0 and dt.x < closest_dist:
			closest_dist = dt.x
			closest_id   = g.get("gate_id", -1) as int
	return closest_id


## Returns the body_id of the celestial body closest to the click ray, or -1.
## Pick radius scales with the body's logical radius (bodies are large
## objects), looked up from `bodies` by body_id.
static func pick_body_at(camera: Camera3D, screen_pos: Vector2, bodies_root: Node, bodies: Array, world_scale: float) -> int:
	var from: Vector3 = camera.project_ray_origin(screen_pos)
	var dir : Vector3 = camera.project_ray_normal(screen_pos)
	var closest_id  : int   = -1
	var closest_dist: float = 1e9
	for marker: Node in bodies_root.get_children():
		if not marker.has_meta("body_id"):
			continue
		var p : Vector3 = (marker as Node3D).global_position
		var dt: Vector2 = ray_point_distance(from, dir, p)
		## Pick radius scales with logical body radius (bodies are large objects).
		var b_radius: float = 0.0
		for entry: Variant in bodies:
			var b: Dictionary = entry as Dictionary
			if (b.get("body_id", -1) as int) == (marker.get_meta("body_id") as int):
				b_radius = (b.get("radius", 1.0) as float) * world_scale * PICK_RADIUS_BODY_SCALE
				break
		var pick_radius: float = maxf(b_radius, PICK_RADIUS_BODY_MIN)
		if dt.x < pick_radius and dt.y > 0.0 and dt.x < closest_dist:
			closest_dist = dt.x
			closest_id   = marker.get_meta("body_id") as int
	return closest_id
