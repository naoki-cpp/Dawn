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
var origin : Vector3 = Vector3.ZERO

## Server-space position -> Godot world position (origin-relative, Z-flipped,
## scaled). The subtraction happens in GDScript floats (f64) before narrowing to
## a Vector3, so a nearby object stays precise even at astronomical magnitudes.
func to_godot(server_pos: Vector3) -> Vector3:
	return Vector3(
		(server_pos.x - origin.x) * WORLD_SCALE,
		(server_pos.y - origin.y) * WORLD_SCALE,
		-(server_pos.z - origin.z) * WORLD_SCALE)

## Exact inverse of `to_godot`: Godot world position -> server-space position.
func to_server(godot_pos: Vector3) -> Vector3:
	return Vector3(
		godot_pos.x / WORLD_SCALE + origin.x,
		godot_pos.y / WORLD_SCALE + origin.y,
		-godot_pos.z / WORLD_SCALE + origin.z)

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
	return player_server.distance_to(origin) >= REBASE_THRESHOLD

## Move the origin to `new_origin` and return the Godot-space delta to add to
## every world node so the move is invisible: a node whose server position is
## unchanged must keep its on-screen place, which means shifting it by
## (old_origin - new_origin) * scale (mirroring `to_godot`'s axis convention).
func rebase_to(new_origin: Vector3) -> Vector3:
	var shift := Vector3(
		(origin.x - new_origin.x) * WORLD_SCALE,
		(origin.y - new_origin.y) * WORLD_SCALE,
		-(origin.z - new_origin.z) * WORLD_SCALE)
	origin = new_origin
	return shift
