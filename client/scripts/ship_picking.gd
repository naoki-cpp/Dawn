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
## Pixel radius for body picking. Unlike PICK_RADIUS_SHIP/GATE (world units,
## measured along the click ray), this is a screen-space threshold: a
## planet's marker stays equally easy to click whether it's near or far,
## since apparent screen size -- not world distance -- determines whether
## the click lands within this many pixels of the marker. See
## screen_point_distance().
const PICK_RADIUS_BODY_PX: float = 10.0


## Perpendicular distance from world point `p` to the ray (`from`, `dir`),
## packed as (dist, t) — `t` is the ray parameter at the closest approach
## (t <= 0 means `p` is behind the camera).
static func ray_point_distance(from: Vector3, dir: Vector3, p: Vector3) -> Vector2:
	var t: float = (p - from).dot(dir)
	return Vector2(p.distance_to(from + dir * t), t)


## Distance in screen pixels between `world_point` projected onto the
## viewport and `screen_pos`, packed as (pixel_dist, front) — `front` is
## positive when `world_point` is in front of the camera, negative when
## behind (unproject_position() gives meaningless results behind the
## camera, so callers must check this before trusting pixel_dist).
static func screen_point_distance(camera: Camera3D, screen_pos: Vector2, world_point: Vector3) -> Vector2:
	var to_point: Vector3 = world_point - camera.global_position
	var forward : Vector3 = -camera.global_transform.basis.z
	var front   : float   = 1.0 if to_point.dot(forward) > 0.0 else -1.0
	var screen_point: Vector2 = camera.unproject_position(world_point)
	return Vector2(screen_point.distance_to(screen_pos), front)


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
## is closest to the click ray, or -1. Picks against the marker's actual
## rendered `global_position` (ADR-0029: gate markers are clamped to a
## bounded render distance like body markers, navigation_marker_renderer.gd /
## main.gd's _update_gate_markers) rather than recomputing the true server
## position, so a click lands on what's on screen even when a gate is too far
## to render at its real position. Gates are large objects, so the pick
## radius is wider than for ships.
static func pick_gate_at(camera: Camera3D, screen_pos: Vector2, gates_root: Node) -> int:
	var from: Vector3 = camera.project_ray_origin(screen_pos)
	var dir : Vector3 = camera.project_ray_normal(screen_pos)
	var closest_id  : int   = -1
	var closest_dist: float = 1e9
	for marker: Node in gates_root.get_children():
		if not marker.has_meta("gate_id"):
			continue
		var p : Vector3 = (marker as Node3D).global_position
		var dt: Vector2 = ray_point_distance(from, dir, p)
		if dt.x < PICK_RADIUS_GATE and dt.y > 0.0 and dt.x < closest_dist:
			closest_dist = dt.x
			closest_id   = marker.get_meta("gate_id") as int
	return closest_id


## Returns the body_id of the celestial body marker closest to the click
## position on screen, or -1. Screen-space (not ray-distance) picking, paired
## with each planet's fixed-screen-size selection reticle
## (navigation_marker_renderer.gd) -- distant planets stay just as easy to
## click as close ones, since both are judged by the same pixel radius.
static func pick_body_at(camera: Camera3D, screen_pos: Vector2, bodies_root: Node) -> int:
	var closest_id  : int   = -1
	var closest_dist: float = 1e9
	for marker: Node in bodies_root.get_children():
		if not marker.has_meta("body_id"):
			continue
		var p : Vector3 = (marker as Node3D).global_position
		var dt: Vector2 = screen_point_distance(camera, screen_pos, p)
		if dt.x < PICK_RADIUS_BODY_PX and dt.y > 0.0 and dt.x < closest_dist:
			closest_dist = dt.x
			closest_id   = marker.get_meta("body_id") as int
	return closest_id
