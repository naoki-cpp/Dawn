## Shared axisymmetric stellar-emissivity and interstellar-extinction model.
class_name GalacticModel
extends RefCounted

const GALACTIC_CENTER_RADIUS_KPC: float = 8.2
const SUN_POSITION_KPC := Vector3(-GALACTIC_CENTER_RADIUS_KPC, 0.023, 0.0)

# The thin/thick disk values are one observation-based Milky Way mass model.
# See Barros et al. (2016), Table 1.
const THIN_DISK_RADIAL_SCALE_KPC: float = 2.12
const THICK_DISK_RADIAL_SCALE_KPC: float = 3.05
const THIN_DISK_SCALE_HEIGHT_KPC: float = 0.205
const THICK_DISK_SCALE_HEIGHT_KPC: float = 0.640
const THICK_DISK_LOCAL_WEIGHT: float = 0.12

# A Gaussian proxy for the inner box/peanut bulge. Its global mass is normalized
# to the adopted bulge-to-total ratio rather than assigned a visual multiplier.
const BULGE_TO_TOTAL_STELLAR_MASS: float = 0.15
const BULGE_ANGLE_RADIANS: float = 0.47
const BULGE_SIGMA_AXES_KPC := Vector3(1.8, 0.42, 0.72)
const SPIRAL_ARM_PITCH_RADIANS: float = 0.21
const SPIRAL_ARM_COUNT: int = 4
const SPIRAL_ARM_WIDTH_KPC: float = 0.42
const SPIRAL_ARM_MIN_RADIUS_KPC: float = 3.5
const SPIRAL_ARM_MAX_RADIUS_KPC: float = 14.0
const SPIRAL_ARM_REFERENCE_RADIUS_KPC: float = 5.0
const SPIRAL_ARM_BASE_ANGLE_RADIANS: float = 0.86

# Dust disk from Li et al. (2018), LAMOST-based model. The local extinction
# normalization is a diffuse-screen approximation, not a 3D cloud map.
const DUST_RADIAL_SCALE_KPC: float = 3.192
const DUST_SCALE_HEIGHT_KPC: float = 0.103
const LOCAL_AV_EXTINCTION_MAG_PER_KPC: float = 0.7
const RGB_EXTINCTION_OVER_AV := Vector3(0.84, 1.0, 1.32)
const MAGNITUDE_TO_OPTICAL_DEPTH: float = 0.921034

# Hayashi (1982) local V-band volume emissivity. Godot's linear colour values
# are not physical radiance units, so the final factor is display adaptation.
const LOCAL_V_LUMINOSITY_DENSITY_LSUN_PC3: float = 0.071
const DISPLAY_RADIANCE_SCALE: float = 0.00372

# Unresolved dust structure is a 3-scale correlated field, normalized before
# applying the log-normal density variance derived from the Mach number.
const DUST_TURBULENCE_OCTAVE_WEIGHTS := Vector3(0.55, 0.30, 0.15)
const DUST_PROJECTED_VARIANCE_FRACTION: float = 0.35
const DUST_CLOUD_ANGULAR_SCALES_RADIANS := Vector3(0.10, 0.035, 0.012)
const STELLAR_MOTTLE_WEIGHTS := Vector2(0.20, 0.11)
# Angular coherence lengths keep the projected unresolved-light field visible
# after the 3D line-of-sight integral, without assigning fake star positions.
const STELLAR_MOTTLE_ANGULAR_SCALES_RADIANS := Vector2(0.10, 0.035)
const DUST_CLOUD_TURBULENCE_DRIVING_PARAMETER: float = 0.4
const DUST_CLOUD_SONIC_MACH_NUMBER: float = 5.0


static func _disk_density_unnormalized(position_kpc: Vector3) -> float:
	var radius := Vector2(position_kpc.x, position_kpc.z).length()
	var height := absf(position_kpc.y)
	var thin := (1.0 - THICK_DISK_LOCAL_WEIGHT) \
		* exp((GALACTIC_CENTER_RADIUS_KPC - radius) / THIN_DISK_RADIAL_SCALE_KPC) \
		* exp(-height / THIN_DISK_SCALE_HEIGHT_KPC)
	var thick := THICK_DISK_LOCAL_WEIGHT \
		* exp((GALACTIC_CENTER_RADIUS_KPC - radius) / THICK_DISK_RADIAL_SCALE_KPC) \
		* exp(-height / THICK_DISK_SCALE_HEIGHT_KPC)
	return thin + thick


static func stellar_disk_density(position_kpc: Vector3) -> float:
	return _disk_density_unnormalized(position_kpc) \
		/ _disk_density_unnormalized(SUN_POSITION_KPC)


static func dust_density(position_kpc: Vector3) -> float:
	var radius := Vector2(position_kpc.x, position_kpc.z).length()
	var radial := exp((GALACTIC_CENTER_RADIUS_KPC - radius) / DUST_RADIAL_SCALE_KPC)
	var vertical := exp((absf(SUN_POSITION_KPC.y) - absf(position_kpc.y)) \
		/ DUST_SCALE_HEIGHT_KPC)
	return radial * vertical


static func dust_cloud_log_density_sigma() -> float:
	return log_density_sigma(
		DUST_CLOUD_TURBULENCE_DRIVING_PARAMETER,
		DUST_CLOUD_SONIC_MACH_NUMBER)


static func dust_volume_log_density_sigma() -> float:
	return dust_cloud_log_density_sigma() * sqrt(1.0 - DUST_PROJECTED_VARIANCE_FRACTION)


static func dust_projected_log_density_sigma() -> float:
	return dust_cloud_log_density_sigma() * sqrt(DUST_PROJECTED_VARIANCE_FRACTION)


static func dust_turbulence_normalization() -> float:
	return interpolated_noise_normalization() \
		/ sqrt(DUST_TURBULENCE_OCTAVE_WEIGHTS.length_squared())


static func stellar_mottle_noise_normalization() -> float:
	return interpolated_noise_normalization()


static func interpolated_noise_normalization() -> float:
	# Hash values are uniform, then trilinearly blended with smoothstep weights.
	# The resulting variance is 1/12 * (26/35)^3, not 1/12.
	var interpolation_variance := (1.0 / 12.0) * pow(26.0 / 35.0, 3.0)
	return 1.0 / sqrt(interpolation_variance)


static func log_density_sigma(driving_parameter: float, sonic_mach_number: float) -> float:
	var b_mach := driving_parameter * sonic_mach_number
	return sqrt(log(1.0 + b_mach * b_mach))


static func visual_extinction(direction: Vector3, distance_kpc: float) -> float:
	const STEPS: int = 96
	var ray := direction.normalized()
	var distance_limit := maxf(distance_kpc, 0.0)
	var av := 0.0
	for i: int in range(STEPS):
		var t0 := float(i) / float(STEPS)
		var t1 := float(i + 1) / float(STEPS)
		var start := t0 * t0 * distance_limit
		var end := t1 * t1 * distance_limit
		var sample_distance := (start + end) * 0.5
		var position := SUN_POSITION_KPC + ray * sample_distance
		av += dust_density(position) * (end - start) \
			* LOCAL_AV_EXTINCTION_MAG_PER_KPC
	return av


static func transmission(visual_extinction_mag: float) -> Color:
	var optical_depth := visual_extinction_mag \
		* MAGNITUDE_TO_OPTICAL_DEPTH * RGB_EXTINCTION_OVER_AV
	return Color(exp(-optical_depth.x), exp(-optical_depth.y), exp(-optical_depth.z))


static func _disk_mass_integral_kpc3() -> float:
	var thin := (1.0 - THICK_DISK_LOCAL_WEIGHT) \
		* exp(GALACTIC_CENTER_RADIUS_KPC / THIN_DISK_RADIAL_SCALE_KPC) \
		* pow(THIN_DISK_RADIAL_SCALE_KPC, 2.0) \
		* THIN_DISK_SCALE_HEIGHT_KPC
	var thick := THICK_DISK_LOCAL_WEIGHT \
		* exp(GALACTIC_CENTER_RADIUS_KPC / THICK_DISK_RADIAL_SCALE_KPC) \
		* pow(THICK_DISK_RADIAL_SCALE_KPC, 2.0) \
		* THICK_DISK_SCALE_HEIGHT_KPC
	return 4.0 * PI * (thin + thick) / _disk_density_unnormalized(SUN_POSITION_KPC)


static func bulge_density_normalization() -> float:
	var bulge_to_disk := BULGE_TO_TOTAL_STELLAR_MASS \
		/ (1.0 - BULGE_TO_TOTAL_STELLAR_MASS)
	var gaussian_volume := pow(2.0 * PI, 1.5) \
		* BULGE_SIGMA_AXES_KPC.x * BULGE_SIGMA_AXES_KPC.y * BULGE_SIGMA_AXES_KPC.z
	return bulge_to_disk * _disk_mass_integral_kpc3() / gaussian_volume


static func configure_material(material: ShaderMaterial) -> void:
	material.set_shader_parameter("galactic_sun_position_kpc", SUN_POSITION_KPC)
	material.set_shader_parameter("galactic_center_radius_kpc", GALACTIC_CENTER_RADIUS_KPC)
	material.set_shader_parameter("thin_disk_radial_scale_kpc", THIN_DISK_RADIAL_SCALE_KPC)
	material.set_shader_parameter("thick_disk_radial_scale_kpc", THICK_DISK_RADIAL_SCALE_KPC)
	material.set_shader_parameter("thin_disk_scale_height_kpc", THIN_DISK_SCALE_HEIGHT_KPC)
	material.set_shader_parameter("thick_disk_scale_height_kpc", THICK_DISK_SCALE_HEIGHT_KPC)
	material.set_shader_parameter("thick_disk_local_weight", THICK_DISK_LOCAL_WEIGHT)
	material.set_shader_parameter("bulge_angle_radians", BULGE_ANGLE_RADIANS)
	material.set_shader_parameter("bulge_sigma_axes_kpc", BULGE_SIGMA_AXES_KPC)
	material.set_shader_parameter(
		"bulge_density_normalization", bulge_density_normalization())
	material.set_shader_parameter(
		"dust_volume_log_density_sigma", dust_volume_log_density_sigma())
	material.set_shader_parameter(
		"dust_projected_log_density_sigma", dust_projected_log_density_sigma())
	material.set_shader_parameter(
		"dust_turbulence_octave_weights", DUST_TURBULENCE_OCTAVE_WEIGHTS)
	material.set_shader_parameter(
		"dust_cloud_angular_scales_radians", DUST_CLOUD_ANGULAR_SCALES_RADIANS)
	material.set_shader_parameter(
		"dust_projected_turbulence_normalization", dust_turbulence_normalization())
	material.set_shader_parameter(
		"dust_turbulence_normalization", dust_turbulence_normalization())
	material.set_shader_parameter("stellar_mottle_weights", STELLAR_MOTTLE_WEIGHTS)
	material.set_shader_parameter(
		"stellar_mottle_angular_scales_radians", STELLAR_MOTTLE_ANGULAR_SCALES_RADIANS)
	material.set_shader_parameter(
		"stellar_mottle_noise_normalization", stellar_mottle_noise_normalization())
	material.set_shader_parameter("dust_scale_length_kpc", DUST_RADIAL_SCALE_KPC)
	material.set_shader_parameter("dust_scale_height_kpc", DUST_SCALE_HEIGHT_KPC)
	material.set_shader_parameter(
		"local_av_extinction_mag_per_kpc", LOCAL_AV_EXTINCTION_MAG_PER_KPC)
	material.set_shader_parameter("rgb_extinction_over_av", RGB_EXTINCTION_OVER_AV)
	material.set_shader_parameter(
		"magnitude_to_optical_depth", MAGNITUDE_TO_OPTICAL_DEPTH)
	material.set_shader_parameter(
		"local_v_luminosity_density_lsun_pc3", LOCAL_V_LUMINOSITY_DENSITY_LSUN_PC3)
	material.set_shader_parameter("display_radiance_scale", DISPLAY_RADIANCE_SCALE)
