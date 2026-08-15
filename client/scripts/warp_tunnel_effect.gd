## warp_tunnel_effect.gd
##
## Thin wrapper around the full-screen WarpTunnel ColorRect's shader
## parameter. WorldPresentation owns the smoothing/threshold logic; this just
## forwards the resulting value.

extends ColorRect

func set_intensity(value: float) -> void:
	(material as ShaderMaterial).set_shader_parameter("intensity", value)

func set_flow_direction(direction: Vector2) -> void:
	var normalized := direction.normalized() if direction.length_squared() > 0.0001 else Vector2(0.0, -1.0)
	(material as ShaderMaterial).set_shader_parameter("flow_direction", normalized)

func set_direction_confidence(value: float) -> void:
	(material as ShaderMaterial).set_shader_parameter("direction_confidence", clampf(value, 0.0, 1.0))
