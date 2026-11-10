#[compute]
#version 450
layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;
layout(rgba16f, set = 0, binding = 1) uniform image2D color_image;
layout(set = 0, binding = 2) uniform sampler2D depth_sampler;
layout(set = 1, binding = 0, std140) uniform LensData {
    vec4 source_color;
    vec4 tint;
    vec4 source; // UV, visibility, physical occlusion radius in pixels
    vec4 shape;  // streak sigma x/y, streak energy fraction, glare fraction
    vec4 glare;  // glare sigma, source variance, integrated source flux, squeeze
    vec4 rays;   // shaft radius, strength, sample count, unused
    vec4 ray_response; // per-step decay, density, occlusion weight, unused
    vec4 reflections; // ghost fraction, ghost sigma, halo fraction, halo radius
} lens;
layout(push_constant, std430) uniform DispatchRegion { vec4 rect; } dispatch_region;
shared float source_visibility;

float psf(vec2 delta, vec2 variance) {
    vec2 normalized = delta * delta / variance;
    if (max(normalized.x, normalized.y) >= 36.0) {
        return 0.0;
    }
    float gaussian = exp(-0.5 * (normalized.x + normalized.y))
        / (6.28318530718 * sqrt(variance.x * variance.y));
    // A resolved disc is not another centrally peaked Gaussian. Blend toward
    // a flat-topped fourth-order kernel with the same covariance and unit integral.
    float q = normalized.x + normalized.y;
    float disc = exp(-q * q / 12.56637061436)
        / (9.86960440109 * sqrt(variance.x * variance.y));
    float source_weight = clamp(lens.glare.y / min(variance.x, variance.y), 0.0, 1.0);
    return mix(gaussian, disc, source_weight * source_weight);
}

// Compact, smooth falloff with no late circular rim: value and slope reach
// zero at the dispatch boundary, while attenuation starts at the source.
float soft_extent(float r) {
    float remaining = max(1.0 - r * r, 0.0);
    return remaining * remaining * remaining;
}

vec3 glare_psf(vec2 delta) {
    vec2 sigma = lens.glare.x * vec2(lens.glare.w, 1.0);
    float inner = psf(delta, sigma * sigma + vec2(lens.glare.y));
    float outer = psf(delta, vec2(pow(lens.glare.x * 5.0, 2.0) + lens.glare.y));
    // Compact diagonal diffraction shoulders, distinct from horizontal coating scatter.
    vec2 diagonal = vec2(delta.x + delta.y, delta.y - delta.x) * 0.70710678;
    vec2 spike_sigma = lens.glare.x * vec2(2.8, 0.25);
    float spikes = 0.5 * (psf(diagonal, spike_sigma * spike_sigma + vec2(lens.glare.y))
        + psf(diagonal.yx, spike_sigma * spike_sigma + vec2(lens.glare.y)));
    return vec3(0.65 * inner + 0.05 * spikes) + 0.3 * lens.tint.rgb * outer;
}

vec3 streak_psf(vec2 delta) {
    // Unit-integral mixture: a long narrow line and shorter, broader coating bands.
    vec2 sigma = lens.shape.xy;
    float core = psf(delta, sigma * sigma + vec2(lens.glare.y));
    vec2 band_sigma = sigma * vec2(0.45, 2.5);
    float bands = psf(delta, band_sigma * band_sigma + vec2(lens.glare.y));
    return lens.tint.rgb * (0.78 * core + 0.22 * bands);
}

vec3 ghost_psf(vec2 point, vec2 source_offset) {
    // Three defocused internal images, sharing a unit-integral energy budget.
    float field = clamp(length(lens.source.xy - 0.5) * 1.41421356, 0.0, 1.0);
    vec2 sigma = lens.reflections.y * vec2(lens.glare.w * 2.5, 0.28);
    vec2 variance = sigma * sigma;
    vec2 offset = vec2(0.0, source_offset.y * 0.25);
    // Area-preserving field deformation keeps authored energy independent of shape.
    float stretch = 1.0 + 0.6 * field * field;
    float angle = 0.35 * (lens.source.x - 0.5) * (lens.source.y - 0.5);
    mat2 rotation = mat2(cos(angle), -sin(angle), sin(angle), cos(angle));
    mat2 field_shape = mat2(1.0 / stretch, 0.0, 0.0, stretch) * rotation;
    return lens.tint.rgb * (0.5 * psf(field_shape * (point + source_offset * 0.65 + offset), variance + vec2(lens.glare.y))
        + 0.3 * psf(field_shape * (point + source_offset * 1.25 - offset), variance * 2.0 + vec2(lens.glare.y))
        + 0.2 * psf(field_shape * (point - source_offset * 0.3), variance * 0.6 + vec2(lens.glare.y)));
}

vec3 halo_psf(vec2 point, vec2 source_offset) {
    // Smooth compact annulus. Substitution u=r^2 makes its integral exactly one.
    float radius = lens.reflections.w;
    float width = sqrt(pow(radius * 0.22, 2.0) + lens.glare.y);
    float inner = max(0.0, radius - width);
    float outer = radius + width;
    float area = outer * outer - inner * inner;
    float field = clamp(length(lens.source.xy - 0.5) * 1.41421356, 0.0, 1.0);
    float stretch = 1.0 + 0.35 * field * field;
    vec2 delta = (point + source_offset) * vec2(1.0 / stretch, stretch);
    float u = (dot(delta, delta) - inner * inner) / area;
    if (u <= 0.0 || u >= 1.0) return vec3(0.0);
    return lens.tint.rgb * (30.0 * u * u * pow(1.0 - u, 2.0) / (3.14159265359 * area));
}

float visible_source(ivec2 pixel) {
    if (any(lessThan(pixel, ivec2(0))) || any(greaterThanEqual(pixel, textureSize(depth_sampler, 0)))) {
        return 0.0;
    }
    return float(texelFetch(depth_sampler, pixel, 0).r <= 0.0);
}

// Screen-space shaft approximation: radial integration of the background mask.
// Unlike camera flare, this is suppressed on foreground surfaces and can remain
// visible beside a partially hidden source. It is not volumetric dust transport.
float light_shafts(ivec2 pixel, vec2 source_pixel, vec2 delta) {
    float distance = length(delta);
    if (lens.rays.y <= 0.0 || distance >= lens.rays.x || visible_source(pixel) <= 0.0) {
        return 0.0;
    }
    int samples = clamp(int(lens.rays.z), 8, 96);
    float jitter = fract(sin(dot(vec2(pixel), vec2(12.9898, 78.233))) * 43758.5453);
    float blocked = 0.0;
    float total = 0.0;
    float weight = 1.0;
    for (int i = 0; i < samples; ++i) {
        float t = (float(i) + jitter) / float(samples) * lens.ray_response.y;
        ivec2 sample_pixel = ivec2(mix(vec2(pixel) + 0.5, source_pixel, t));
        blocked += (1.0 - visible_source(sample_pixel)) * weight;
        total += weight;
        weight *= lens.ray_response.x;
    }
    // ARez-style decayed occlusion integration followed by a squared inverse
    // mask. Normalize weights so changing sample quality does not change gain.
    float mask = 1.0 - clamp(blocked / total * lens.ray_response.z, 0.0, 1.0);
    float r = distance / lens.rays.x;
    float glow = 1.0 / pow(1.0 + 6.0 * r, 2.0);
    return mask * mask * glow
        * soft_extent(r) * lens.rays.y;
}

void main() {
    ivec2 size = imageSize(color_image);
    vec2 source_uv = lens.source.xy;
    if (lens.source.z <= 0.0 || any(lessThan(source_uv, vec2(0.0)))
        || any(greaterThanEqual(source_uv, vec2(1.0)))) {
        return;
    }
    // Physical source visibility is shared per workgroup, not sampled along
    // the streak: lens artifacts can overlay a foreground hull.
    if (gl_LocalInvocationIndex == 0) {
        ivec2 center = ivec2(source_uv * vec2(size));
        int spread = max(1, int(lens.source.w));
        float visible = visible_source(center);
        const ivec2 offsets[8] = ivec2[8](ivec2(1,0), ivec2(-1,0), ivec2(0,1), ivec2(0,-1),
            ivec2(1,1), ivec2(-1,1), ivec2(1,-1), ivec2(-1,-1));
        for (int i = 0; i < 8; ++i) {
            visible += visible_source(center + offsets[i] * spread);
        }
        source_visibility = visible / 9.0;
        if (lens.source.w > 12.0) {
            // Equal-area disc samples: an occulted centre must not extinguish
            // the whole resolved star. Keep work bounded for a screen-sized disc.
            visible = 0.0;
            for (int i = 0; i < 48; ++i) {
                float r = sqrt((float(i) + 0.5) / 48.0) * lens.source.w;
                float angle = float(i) * 2.39996323;
                visible += visible_source(ivec2(source_uv * vec2(size)
                    + r * vec2(cos(angle), sin(angle))));
            }
            source_visibility = visible / 48.0;
        }
    }
    barrier();
    ivec2 local = ivec2(gl_GlobalInvocationID.xy);
    ivec2 pixel = local + ivec2(dispatch_region.rect.xy);
    if (any(greaterThanEqual(local, ivec2(dispatch_region.rect.zw)))
        || any(greaterThanEqual(pixel, size)) || (source_visibility <= 0.0 && lens.rays.y <= 0.0)) {
        return;
    }
    vec2 delta = (vec2(pixel) + 0.5 - source_uv * vec2(size)) / float(size.y);
    // Second-moment disc convolution: the same source variance broadens every
    // normalized PSF component. Their energy weights, not peaks, are authored.
    vec2 point = (vec2(pixel) + 0.5 - vec2(size) * 0.5) / float(size.y);
    vec2 source_offset = (source_uv - 0.5) * vec2(size) / float(size.y);
    vec3 kernel = lens.shape.w * glare_psf(delta) + lens.shape.z * streak_psf(delta);
    if (lens.reflections.x > 0.0) kernel += lens.reflections.x * ghost_psf(point, source_offset);
    if (lens.reflections.z > 0.0) kernel += lens.reflections.z * halo_psf(point, source_offset);
    vec4 original = imageLoad(color_image, pixel);
    vec3 optical_light = lens.source_color.rgb * lens.source_color.a
        * lens.glare.z * kernel * lens.source.z * source_visibility;
    // Shafts remain a separate screen-space effect, not a second lens PSF.
    optical_light += lens.source_color.rgb * lens.tint.rgb * lens.source_color.a
        * lens.source.z * light_shafts(pixel, source_uv * vec2(size), delta);
    imageStore(color_image, pixel, vec4(original.rgb + optical_light, original.a));
}
