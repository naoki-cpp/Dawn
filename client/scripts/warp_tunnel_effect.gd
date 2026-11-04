## warp_tunnel_effect.gd
##
## Shader adapter for the travel-aligned tunnel. WorldPresentation owns the
## intensity smoothing; this forwards motion and camera data for projection.

extends ColorRect

func set_intensity(value: float) -> void:
	(material as ShaderMaterial).set_shader_parameter("intensity", value)

func set_motion(velocity: Vector3, view_basis: Basis, fov: float) -> void:
	var shader_material := material as ShaderMaterial
	# Keep the last travel axis during the arrival fade instead of snapping it.
	if velocity.length_squared() > 0.0001:
		shader_material.set_shader_parameter("travel_direction", velocity.normalized())
	shader_material.set_shader_parameter("camera_basis", view_basis)
	shader_material.set_shader_parameter("camera_fov", fov)
