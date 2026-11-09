#[compute]
#version 450
layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;
layout(rgba16f, set = 0, binding = 1) uniform image2D color_image;
layout(set = 0, binding = 2) uniform sampler2D depth_sampler;
layout(set = 1, binding = 0, std140) uniform LensData {
    vec4 tint;
    vec4 source; // UV, visibility, physical occlusion radius in pixels
    vec4 shape;  // horizontal decay length, vertical width, intensity, unused
    vec4 glare;  // radial falloff scale, strength, unused, unused
    vec4 rays;   // shaft radius, strength, sample count, unused
    vec4 ray_response; // per-step decay, density, occlusion weight, unused
} lens;
layout(push_constant, std430) uniform DispatchRegion { vec4 rect; } dispatch_region;
shared float source_visibility;

// Compact, smooth falloff with no late circular rim: value and slope reach
// zero at the dispatch boundary, while attenuation starts at the source.
float soft_extent(float r) {
    float remaining = max(1.0 - r * r, 0.0);
    return remaining * remaining * remaining;
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
    }
    barrier();
    ivec2 local = ivec2(gl_GlobalInvocationID.xy);
    ivec2 pixel = local + ivec2(dispatch_region.rect.xy);
    if (any(greaterThanEqual(local, ivec2(dispatch_region.rect.zw)))
        || any(greaterThanEqual(pixel, size)) || (source_visibility <= 0.0 && lens.rays.y <= 0.0)) {
        return;
    }
    vec2 delta = (vec2(pixel) + 0.5 - source_uv * vec2(size)) / float(size.y);
    float x = abs(delta.x) / lens.shape.x;
    // Narrow long tail plus a brighter, slightly wider response near the source.
    // This is an anisotropic point-spread approximation, not optical ray tracing.
    float width = lens.shape.y * (1.0 + 0.8 * exp(-4.0 * x));
    float y = delta.y / width;
    float response = (0.7 * exp(-4.0 * x) + 0.3 * exp(-x)) * exp(-0.5 * y * y);
    response *= 1.0 - smoothstep(4.5, 6.0, x);
    response *= 1.0 - smoothstep(4.5, 6.0, abs(y));
    vec4 original = imageLoad(color_image, pixel);
    // Smooth camera glare, not another opaque solar disc. A strong inner
    // falloff joins a much fainter extended veil without a circular boundary.
    float r = length(delta) / lens.glare.x;
    float glare = (exp(-2.0 * r) + 0.012 * exp(-0.5 * r))
        * soft_extent(r / 6.0) * lens.glare.y;
    vec3 glare_tint = mix(lens.tint.rgb, vec3(1.0, 0.93, 0.72), exp(-r));
    vec3 streak = (lens.tint.rgb * lens.shape.z * response + glare_tint * glare)
        * lens.tint.a * lens.source.z * source_visibility;
    streak += lens.tint.rgb * lens.tint.a * lens.source.z
        * light_shafts(pixel, source_uv * vec2(size), delta);
    // One scene-linear addition; glow and display mapping belong to Godot.
    imageStore(color_image, pixel, vec4(original.rgb + streak, original.a));
}
