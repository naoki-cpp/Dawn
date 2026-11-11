uniform vec3 galactic_sun_position_kpc = vec3(-8.2, 0.023, 0.0);
uniform float galactic_center_radius_kpc = 8.2;
uniform float thin_disk_radial_scale_kpc = 2.12;
uniform float thick_disk_radial_scale_kpc = 3.05;
uniform float thin_disk_scale_height_kpc = 0.205;
uniform float thick_disk_scale_height_kpc = 0.640;
uniform float thick_disk_local_weight = 0.12;
uniform float bulge_angle_radians = 0.47;
uniform vec3 bulge_sigma_axes_kpc = vec3(1.8, 0.42, 0.72);
uniform float bulge_density_normalization = 12.0;
uniform float dust_scale_length_kpc = 3.192;
uniform float dust_scale_height_kpc = 0.103;
uniform float local_av_extinction_mag_per_kpc = 0.7;
uniform float magnitude_to_optical_depth = 0.921034;
uniform vec3 rgb_extinction_over_av = vec3(0.84, 1.0, 1.32);
uniform float local_v_luminosity_density_lsun_pc3 = 0.071;
uniform float display_radiance_scale = 0.00372;
uniform float dust_volume_log_density_sigma = 1.027;
uniform float dust_projected_log_density_sigma = 0.751;
uniform vec3 dust_turbulence_octave_weights = vec3(0.55, 0.30, 0.15);
uniform float dust_turbulence_normalization = 8.401;
uniform vec3 dust_cloud_angular_scales_radians = vec3(0.36, 0.12, 0.04);
uniform float dust_projected_turbulence_normalization = 8.401;
uniform vec2 stellar_mottle_weights = vec2(0.20, 0.11);
uniform vec2 stellar_mottle_angular_scales_radians = vec2(0.24, 0.08);
uniform float stellar_mottle_noise_normalization = 5.41;

vec3 to_galactic(vec3 direction) {
	// SkyCatalog stores equatorial vectors as (cos(dec)cos(ra), sin(dec),
	// cos(dec)sin(ra)); the standard ICRS->Galactic matrix uses XYZ instead.
	vec3 equatorial = vec3(direction.x, direction.z, direction.y);
	vec3 standard_galactic = vec3(
		dot(vec3(-0.05487556, -0.87343709, -0.48383502), equatorial),
		dot(vec3(0.49410943, -0.44482963, 0.74698224), equatorial),
		dot(vec3(-0.86766615, -0.19807637, 0.45598378), equatorial)
	);
	return vec3(standard_galactic.x, standard_galactic.z, standard_galactic.y);
}

float galactic_stellar_disk_density(vec3 position_kpc) {
	float radius = length(position_kpc.xz);
	float height = abs(position_kpc.y);
	float thin_disk = (1.0 - thick_disk_local_weight)
		* exp((galactic_center_radius_kpc - radius) / thin_disk_radial_scale_kpc)
		* exp(-height / thin_disk_scale_height_kpc);
	float thick_disk = thick_disk_local_weight
		* exp((galactic_center_radius_kpc - radius) / thick_disk_radial_scale_kpc)
		* exp(-height / thick_disk_scale_height_kpc);
	float local_height = abs(galactic_sun_position_kpc.y);
	float local_thin = (1.0 - thick_disk_local_weight)
		* exp(-local_height / thin_disk_scale_height_kpc);
	float local_thick = thick_disk_local_weight
		* exp(-local_height / thick_disk_scale_height_kpc);
	return (thin_disk + thick_disk) / (local_thin + local_thick);
}

float galactic_bulge_density(vec3 position_kpc) {
	float c = cos(bulge_angle_radians);
	float s = sin(bulge_angle_radians);
	vec3 bar_position = vec3(
		position_kpc.x * c + position_kpc.z * s,
		position_kpc.y,
		-position_kpc.x * s + position_kpc.z * c
	);
	float sigma_radius = length(bar_position / bulge_sigma_axes_kpc);
	return bulge_density_normalization * exp(-0.5 * sigma_radius * sigma_radius);
}

vec3 galactic_turbulence_octaves(vec3 position_kpc) {
	return vec3(
		snoise(position_kpc * 0.55 + vec3(17.3, 5.7, 31.1)),
		snoise(position_kpc * 1.4 + vec3(43.1, 29.7, 11.9)),
		snoise(position_kpc * 3.7 + vec3(7.9, 53.3, 23.5))
	);
}

float galactic_turbulence_normal(vec3 octave_samples) {
	return dot(octave_samples - vec3(0.5), dust_turbulence_octave_weights)
		* dust_turbulence_normalization;
}

float galactic_projected_stellar_mottling(vec3 galactic_direction) {
	float broad = snoise(galactic_direction / stellar_mottle_angular_scales_radians.x
		+ vec3(17.3, 5.7, 31.1));
	float middle = snoise(galactic_direction / stellar_mottle_angular_scales_radians.y
		+ vec3(43.1, 29.7, 11.9));
	vec2 fields = (vec2(broad, middle) - vec2(0.5))
		* stellar_mottle_noise_normalization;
	return clamp(1.0 + dot(fields, stellar_mottle_weights), 0.25, 1.95);
}

float galactic_projected_dust_turbulence(vec3 galactic_direction) {
	vec3 samples = vec3(
		snoise(galactic_direction / dust_cloud_angular_scales_radians.x
			+ vec3(7.9, 53.3, 23.5)),
		snoise(galactic_direction / dust_cloud_angular_scales_radians.y
			+ vec3(17.3, 5.7, 31.1)),
		snoise(galactic_direction / dust_cloud_angular_scales_radians.z
			+ vec3(43.1, 29.7, 11.9))
	);
	return dot(samples - vec3(0.5), dust_turbulence_octave_weights)
		* dust_projected_turbulence_normalization;
}

float galactic_lognormal_density(float normal_sample, float log_density_sigma) {
	float variance = log_density_sigma * log_density_sigma;
	return exp(log_density_sigma * normal_sample - 0.5 * variance);
}

float galactic_dust_density(vec3 position_kpc) {
	float radius = length(position_kpc.xz);
	float radial = exp((galactic_center_radius_kpc - radius) / dust_scale_length_kpc);
	float vertical = exp((abs(galactic_sun_position_kpc.y)
		- abs(position_kpc.y)) / dust_scale_height_kpc);
	return radial * vertical;
}

float galactic_visual_extinction(vec3 galactic_direction, float distance_kpc) {
	const int steps = 32;
	float distance_limit = max(distance_kpc, 0.0);
	float visual_extinction_mag = 0.0;
	float projected_cloud_factor = galactic_lognormal_density(
		galactic_projected_dust_turbulence(galactic_direction),
		dust_projected_log_density_sigma);
	for (int i = 0; i < steps; i++) {
		float t0 = float(i) / float(steps);
		float t1 = float(i + 1) / float(steps);
		float start = t0 * t0 * distance_limit;
		float end = t1 * t1 * distance_limit;
		float step_kpc = end - start;
		float distance = (start + end) * 0.5;
		vec3 position = galactic_sun_position_kpc + galactic_direction * distance;
		visual_extinction_mag += galactic_dust_density(position) * projected_cloud_factor
			* step_kpc * local_av_extinction_mag_per_kpc;
	}
	return visual_extinction_mag;
}

vec3 extinction_transmission(float visual_extinction_mag) {
	// A_lambda is measured in magnitudes; convert it to an intensity ratio.
	return exp(-magnitude_to_optical_depth
		* visual_extinction_mag * rgb_extinction_over_av);
}

vec3 galactic_blackbody_rgb(float temperature_kelvin) {
	const vec3 wavelength_m = vec3(650e-9, 550e-9, 450e-9);
	const float second_radiation_constant_m_kelvin = 1.438776877e-2;
	vec3 exponent = second_radiation_constant_m_kelvin
		/ (wavelength_m * temperature_kelvin);
	vec3 spectral_radiance = 1.0
		/ (pow(wavelength_m, vec3(5.0)) * (exp(exponent) - vec3(1.0)));
	return spectral_radiance / spectral_radiance.g;
}

vec3 galactic_integrated_radiance(vec3 galactic_direction, float extinction_scale) {
	const int steps = 128;
	const float maximum_distance_kpc = 22.0;
	vec3 disk_radiance = vec3(0.0);
	vec3 bulge_radiance = vec3(0.0);
	float visual_extinction_mag = 0.0;
	vec3 stellar_spectrum = galactic_blackbody_rgb(5000.0);
	float projected_dust_normal = galactic_projected_dust_turbulence(galactic_direction);
	float projected_dust_factor = galactic_lognormal_density(
		projected_dust_normal, dust_projected_log_density_sigma);
	for (int i = 0; i < steps; i++) {
		float t0 = float(i) / float(steps);
		float t1 = float(i + 1) / float(steps);
		float start_kpc = t0 * t0 * maximum_distance_kpc;
		float end_kpc = t1 * t1 * maximum_distance_kpc;
		float step_kpc = end_kpc - start_kpc;
		float distance = (start_kpc + end_kpc) * 0.5;
		vec3 position = galactic_sun_position_kpc + galactic_direction * distance;
		vec3 turbulence_octaves = galactic_turbulence_octaves(position);
		float turbulence = galactic_turbulence_normal(turbulence_octaves);
		float disk_density = galactic_stellar_disk_density(position);
		float bulge_density = galactic_bulge_density(position);
		float clumpy_dust_density = galactic_dust_density(position)
			* projected_dust_factor
			* galactic_lognormal_density(turbulence, dust_volume_log_density_sigma);
		float delta_av = clumpy_dust_density * step_kpc
			* local_av_extinction_mag_per_kpc;
		vec3 transmission = extinction_transmission(
			(visual_extinction_mag + delta_av * 0.5) * extinction_scale);
		float path_length_pc = step_kpc * 1000.0;
		vec3 emitted = transmission * stellar_spectrum
			* local_v_luminosity_density_lsun_pc3 * path_length_pc / (4.0 * PI);
		disk_radiance += emitted * disk_density;
		bulge_radiance += emitted * bulge_density;

		visual_extinction_mag += delta_av;
	}
	return (disk_radiance * galactic_projected_stellar_mottling(galactic_direction)
		+ bulge_radiance) * display_radiance_scale;
}
