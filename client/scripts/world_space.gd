## world_space.gd
##
## The client's single coordinate authority (ADR-0029 #3). It owns the *floating
## origin* — the one server-space point currently mapped to the Godot world
## origin — and is the only place that converts between server space (Y-up, +Z,
## metres-ish) and Godot world space (Y-up, -Z, scaled by WORLD_SCALE).
##
## Why one object: server positions can sit at true astronomical distances (a
## planet at ~7.5e11 m). A Godot Transform3D stores f32, whose ulp there is tens
## of km, so rendering relative to a fixed origin jitters. Keeping a floating
## origin near the player means every object renders at `(server - origin) *
## scale`, so nearby objects have small, precise coordinates regardless of the
## absolute magnitude. The origin only rebases when the player drifts past a
## threshold; on rebase every world node shifts by the same delta, so relative
## positions are preserved exactly (the spike's C2-2 property — no visible jump).
##
## Before this object the origin lived as a bare Vector3 in main.gd and the
## inverse transform was re-derived ad hoc as `global_position / WORLD_SCALE`
## (with a hand-written Z flip) in half a dozen call sites. Those agree with the
## real inverse only while the origin is [0,0,0]; the moment it moves (real AU)
## they silently disagree. Routing every conversion through `to_godot` /
## `to_server` here makes the forward and inverse transforms provably mutual.
##
## At the current compressed scale the whole system fits inside REBASE_THRESHOLD,
## so the origin never leaves [0,0,0] and every transform below is identical to
## the pre-floating-origin behaviour. It activates only once data moves to real AU.
class_name WorldSpace
extends RefCounted

## Server-to-Godot coordinate scale factor. The single source of truth for the
## position scale; ship_controller keeps its own copy only for velocity vectors
## (which carry no origin and so don't go through this object).
const WORLD_SCALE : float = 0.1

## Rebase the origin once the player drifts this far (server units) from it.
## Larger than the compressed system (±700k) so it is dormant until real-AU.
const REBASE_THRESHOLD : float = 1_000_000.0

## The server-space point currently rendered at the Godot world origin.
## Keep the authoritative origin in scalar f64 GDScript floats. A Vector3 is
## f32, so storing an AU-scale origin there would reintroduce the precision loss
## this floating-origin object is meant to avoid.
var _origin_x : float = 0.0
var _origin_y : float = 0.0
var _origin_z : float = 0.0

var origin: Vector3:
	## Compatibility view only. New code must use component methods because this
	## getter/setter necessarily narrows an AU-scale value to Vector3.
	get:
		return Vector3(_origin_x, _origin_y, _origin_z)
	set(value):
		_origin_x = value.x
		_origin_y = value.y
		_origin_z = value.z

## Server-space position -> Godot world position (origin-relative, Z-flipped,
## scaled). The subtraction happens in GDScript floats (f64) before narrowing to
## a Vector3, so a nearby object stays precise even at astronomical magnitudes.
## Compatibility wrapper for legacy Vector3 payloads; absolute wire positions
## must use `to_godot_components` instead.
func to_godot(server_pos: Vector3) -> Vector3:
	return to_godot_components(server_pos.x, server_pos.y, server_pos.z)

## f64-safe variant for absolute wire positions. Do not construct a Vector3 from
## an AU-scale payload before calling this method: that would quantize the
## payload before the origin subtraction.
func to_godot_components(server_x: float, server_y: float, server_z: float) -> Vector3:
	return Vector3(
		(server_x - _origin_x) * WORLD_SCALE,
		(server_y - _origin_y) * WORLD_SCALE,
		-(server_z - _origin_z) * WORLD_SCALE)

## Exact inverse of `to_godot`: Godot world position -> server-space position.
## This compatibility wrapper returns a narrowed Vector3; use
## `to_server_components` for authoritative coordinates.
func to_server(godot_pos: Vector3) -> Vector3:
	var precise := to_server_components(godot_pos)
	return Vector3(precise[0], precise[1], precise[2])

## Inverse transform that preserves the f64 server-space result until a caller
## explicitly chooses to narrow it to a Godot Vector3.
func to_server_components(godot_pos: Vector3) -> PackedFloat64Array:
	return PackedFloat64Array([
		godot_pos.x / WORLD_SCALE + _origin_x,
		godot_pos.y / WORLD_SCALE + _origin_y,
		-godot_pos.z / WORLD_SCALE + _origin_z])

## Direction (not position) server -> Godot: Z flip and scale, no origin offset.
## Magnitude is rarely meaningful (callers usually normalize); the value matters
## only for its bearing.
func dir_to_godot(server_dir: Vector3) -> Vector3:
	return Vector3(server_dir.x, server_dir.y, -server_dir.z) * WORLD_SCALE

## Direction (not position) Godot -> server: Z flip and unscale, no origin offset.
func dir_to_server(godot_dir: Vector3) -> Vector3:
	return Vector3(godot_dir.x, godot_dir.y, -godot_dir.z) / WORLD_SCALE

## Whether the player has drifted far enough from the origin that it should
## rebase to keep render coordinates small (and f32-precise).
func should_rebase(player_server: Vector3) -> bool:
	## Compatibility wrapper for legacy Vector3 callers.
	return should_rebase_components(player_server.x, player_server.y, player_server.z)

func should_rebase_components(player_x: float, player_y: float, player_z: float) -> bool:
	var dx := player_x - _origin_x
	var dy := player_y - _origin_y
	var dz := player_z - _origin_z
	return sqrt(dx * dx + dy * dy + dz * dz) >= REBASE_THRESHOLD

## Distance between two absolute positions while keeping all arithmetic in
## scalar f64 values. Callers should use this for range and proximity checks
## instead of narrowing either position to Vector3 first.
func distance_components(first: PackedFloat64Array, second: PackedFloat64Array) -> float:
	var dx := first[0] - second[0]
	var dy := first[1] - second[1]
	var dz := first[2] - second[2]
	return sqrt(dx * dx + dy * dy + dz * dz)

## Move the origin to `new_origin` and return the Godot-space delta to add to
## every world node so the move is invisible: a node whose server position is
## unchanged must keep its on-screen place, which means shifting it by
## (old_origin - new_origin) * scale (mirroring `to_godot`'s axis convention).
func rebase_to(new_origin: Vector3) -> Vector3:
	## Compatibility wrapper for legacy Vector3 callers.
	return rebase_to_components(new_origin.x, new_origin.y, new_origin.z)

func rebase_to_components(new_x: float, new_y: float, new_z: float) -> Vector3:
	var shift := Vector3(
		(_origin_x - new_x) * WORLD_SCALE,
		(_origin_y - new_y) * WORLD_SCALE,
		-(_origin_z - new_z) * WORLD_SCALE)
	_origin_x = new_x
	_origin_y = new_y
	_origin_z = new_z
	return shift
