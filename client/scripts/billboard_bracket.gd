## billboard_bracket.gd
##
## Procedural EVE-style navigation bracket. The four open corners read as a
## selectable object without covering the physical body underneath. It is a
## fixed-size billboard so the navigation language stays stable at distance.
class_name BillboardBracket
extends RefCounted

const TEXTURE_PX: int = 128
const HALF_EXTENT: float = 0.35
const SEGMENT_LENGTH: float = 0.15
const LINE_WIDTH: float = 0.016
const AA_WIDTH: float = 0.010
## Screen-space tolerance shared by the marker parent and ShipPicking. The
## bracket is decorative geometry, so selection belongs to the marker root.
const PICK_RADIUS_PX: float = 16.0

static var _texture: ImageTexture = null


static func _segment_alpha(point: Vector2, start: Vector2, finish: Vector2) -> float:
	var edge := finish - start
	var edge_length_sq := edge.length_squared()
	var progress := 0.0 if edge_length_sq <= 0.000001 else clampf(
		(point - start).dot(edge) / edge_length_sq, 0.0, 1.0)
	var nearest := start + edge * progress
	var distance := point.distance_to(nearest)
	return clampf((LINE_WIDTH + AA_WIDTH - distance) / AA_WIDTH, 0.0, 1.0)


static func _bracket_alpha(point: Vector2) -> float:
	var alpha := 0.0
	for sx: float in [-1.0, 1.0]:
		for sy: float in [-1.0, 1.0]:
			var corner := Vector2(sx * HALF_EXTENT, sy * HALF_EXTENT)
			alpha = maxf(alpha, _segment_alpha(
				point, corner, corner - Vector2(sx * SEGMENT_LENGTH, 0.0)))
			alpha = maxf(alpha, _segment_alpha(
				point, corner, corner - Vector2(0.0, sy * SEGMENT_LENGTH)))
	return alpha


static func _get_texture() -> ImageTexture:
	if _texture != null:
		return _texture
	var image := Image.create(TEXTURE_PX, TEXTURE_PX, false, Image.FORMAT_RGBA8)
	for y: int in range(TEXTURE_PX):
		for x: int in range(TEXTURE_PX):
			var point := Vector2(
				(float(x) + 0.5) / float(TEXTURE_PX) - 0.5,
				(float(y) + 0.5) / float(TEXTURE_PX) - 0.5)
			image.set_pixel(x, y, Color(1.0, 1.0, 1.0, _bracket_alpha(point)))
	_texture = ImageTexture.create_from_image(image)
	return _texture


## Builds a fixed-size, depth-independent navigation bracket.
static func build(color: Color, pixel_size: float) -> Sprite3D:
	var sprite := Sprite3D.new()
	sprite.texture = _get_texture()
	sprite.fixed_size = true
	sprite.pixel_size = pixel_size
	sprite.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	sprite.no_depth_test = true
	sprite.texture_filter = BaseMaterial3D.TEXTURE_FILTER_LINEAR
	sprite.modulate = color
	return sprite
