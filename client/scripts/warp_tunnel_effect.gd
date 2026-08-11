## warp_tunnel_effect.gd
##
## Thin wrapper around the full-screen WarpTunnel ColorRect's shader
## parameter. WorldPresentation owns the smoothing/threshold logic; this just
## forwards the resulting value.

extends ColorRect

func set_intensity(value: float) -> void:
	(material as ShaderMaterial).set_shader_parameter("intensity", value)
