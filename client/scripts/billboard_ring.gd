## billboard_ring.gd
##
## Shared "selection ring" billboard: a small ring Sprite3D with fixed_size
## enabled, so it renders at a constant on-screen size regardless of camera
## distance. Anything communicating "this is selected/selectable" in 3D
## world space should look and behave the same way -- the planet/gate
## marker reticle (navigation_marker_renderer.gd) and the ship lock-on ring
## (ship_controller.gd) are the same kind of indicator, so this is the one
## place that builds it, instead of each file growing its own copy with
## its own (possibly inconsistent) distance behavior.
##
## The ring texture is generated procedurally (no external image asset,
## matching space_sky.gdshader's convention) with anti-aliased edges: a
## hard 0/1 cutoff at low resolution looks visibly blocky once fixed_size
## scales it down to UI size on screen.
class_name BillboardRing
extends RefCounted

const TEXTURE_PX  : int   = 128
const OUTER_RATIO : float = 0.46
const INNER_RATIO : float = 0.36
## Width of the anti-aliased falloff at each edge, in texture pixels.
const AA_PX       : float = 1.5

static var _texture: ImageTexture = null


## Procedurally builds (and caches) the ring texture. Plain white -- callers
## tint it via the returned Sprite3D's `modulate` instead of baking a colour
## into the texture, so one cached texture serves every caller/colour.
static func _get_texture() -> ImageTexture:
	if _texture != null:
		return _texture
	var size: int = TEXTURE_PX
	var img: Image = Image.create(size, size, false, Image.FORMAT_RGBA8)
	var center  : Vector2 = Vector2(size, size) * 0.5
	var outer_r : float   = size * OUTER_RATIO
	var inner_r : float   = size * INNER_RATIO
	for y: int in range(size):
		for x: int in range(size):
			var d: float = Vector2(x + 0.5, y + 0.5).distance_to(center)
			## Smooth falloff at both edges instead of a hard cutoff.
			var a_outer: float = clampf((outer_r - d) / AA_PX, 0.0, 1.0)
			var a_inner: float = clampf((d - inner_r) / AA_PX, 0.0, 1.0)
			var a: float = minf(a_outer, a_inner)
			img.set_pixel(x, y, Color(1.0, 1.0, 1.0, a))
	_texture = ImageTexture.create_from_image(img)
	return _texture


## Builds a ready-to-place selection ring: fixed screen size, billboarded,
## tinted `color`. `pixel_size` controls the apparent on-screen size (see
## SpriteBase3D.pixel_size) -- size it against existing HUD elements
## (hud_manager.gd), not 3D-world scale, since the whole point of fixed_size
## is that world distance no longer matters.
static func build(color: Color, pixel_size: float) -> Sprite3D:
	var sprite := Sprite3D.new()
	sprite.texture        = _get_texture()
	sprite.fixed_size     = true
	sprite.pixel_size     = pixel_size
	sprite.billboard       = BaseMaterial3D.BILLBOARD_ENABLED
	sprite.no_depth_test   = true
	sprite.texture_filter  = BaseMaterial3D.TEXTURE_FILTER_LINEAR
	sprite.modulate        = color
	return sprite
