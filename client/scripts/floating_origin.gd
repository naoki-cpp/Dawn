## floating_origin.gd
##
## Floating-origin coordinate transform for true-scale rendering (ADR-0029
## step 6, client C2). Server positions can sit at true astronomical distances
## (a planet at ~7.5e11 m); a standard Godot build stores Transform3D in f32,
## whose ulp at that magnitude is tens of km, so rendering relative to a fixed
## origin jitters. Instead we keep a *floating origin* near the player: every
## object renders at `(server_pos - origin) * world_scale`, so nearby objects
## have small, precise coordinates regardless of the absolute magnitude.
##
## The origin only moves (rebases) when the player drifts past a threshold from
## it; on rebase every world node shifts by the same delta, so relative
## positions are preserved exactly (the spike's C2-2 result: camera and nearby
## objects share one origin, so there is no visible jump). Far objects still
## quantise in f32 — that is handled by drawing distant bodies as billboards /
## markers (NavigationMarkerRenderer), not precise meshes (spike S5 finding).
##
## At the current compressed scale the whole system fits within ±700k units, so
## with a large threshold the origin never leaves [0,0,0] and this is a no-op —
## it activates only once data moves to real AU.
class_name FloatingOrigin
extends RefCounted

## Convert a server-space position (Y-up, +Z) to Godot world space (Y-up, -Z),
## relative to `origin` (server space), applying `world_scale`. Subtraction is
## done in GDScript floats (f64) before the result narrows to a Vector3, so a
## nearby object stays precise even when `server_pos`/`origin` are astronomical.
static func to_godot(server_pos: Vector3, origin: Vector3, world_scale: float) -> Vector3:
	return Vector3(
		(server_pos.x - origin.x) * world_scale,
		(server_pos.y - origin.y) * world_scale,
		-(server_pos.z - origin.z) * world_scale)

## Whether the player has drifted far enough from the current origin that the
## origin should rebase to keep render coordinates small.
static func should_rebase(player_server: Vector3, origin: Vector3, threshold: float) -> bool:
	return player_server.distance_to(origin) >= threshold

## The Godot-space shift to apply to every world node when the origin moves from
## `old_origin` to `new_origin`. Adding this to each node's `global_position`
## keeps every object in the same place relative to the (also-shifted) player,
## so the rebase is invisible. Mirrors `to_godot`'s axis/scale convention with
## no extra translation (it is a pure delta).
static func rebase_shift(old_origin: Vector3, new_origin: Vector3, world_scale: float) -> Vector3:
	return Vector3(
		(old_origin.x - new_origin.x) * world_scale,
		(old_origin.y - new_origin.y) * world_scale,
		-(old_origin.z - new_origin.z) * world_scale)
