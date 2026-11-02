#[compute]
#version 450
layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

// ARez: This is the lens flare shader I used: https://www.shadertoy.com/view/wlcyzj

// Samples 9 positions around the sun, instead of just the sun pixel to determine
// how visible the sun is.
#define EXPENSIVE_OCCLUSION_TEST
// Skips 2 "smoothstep" calls
// #define CHEAP_FLARE

// Color image uniform (binding = 1) should be defined separately with restrict
// writeonly/readonly params.
layout(set = 0, binding = 2) uniform sampler2D depth_sampler;

highp float length_squared(vec2 v) {
    return pow(v.x, 2.0) + pow(v.y, 2.0);
}

highp float get_raw_depth(ivec2 p_coord) {
    if (any(lessThan(p_coord, ivec2(0)))
        || any(greaterThanEqual(p_coord, textureSize(depth_sampler, 0)))) {
        return 1.0;
    }
    return texelFetch(depth_sampler, p_coord, 0).r;
}

highp float random(vec2 uv) {
    return fract(sin(dot(uv.xy, vec2(12.9898, 78.233))) * 43758.5453123);
}
// 2x1 hash. Used to jitter the samples.
highp float hash(vec2 p) {
    return fract(sin(dot(p, vec2(41, 289))) * 45758.5453);
}

struct EffectData {
    vec4 sun_color;
    highp float sun_position_x;
    highp float sun_position_y;
    highp float dir_mult; // 1 if looking towards the sun, -1 else
    highp float anamorphic_treshold;
    highp float anamorphic_intensity;
    highp float anamorphic_stretch;
    highp float anamorphic_brightness;
    highp float effect_multiplier;
    highp float effect_easing;

    // God ray uniforms
    highp float decay;
    highp float density;
    highp float weight;
    highp float sample_count;
    highp float occlusion_radius;
    highp float effect_radius;
};

layout(set = 1, binding = 0, std140) uniform EffectDataBlock {
    EffectData data;
} datablock;

layout(rgba16f, set = 0, binding = 1) uniform image2D color_image;
shared float source_visibility;
layout(push_constant, std430) uniform DispatchRegion {
    vec4 rect;
} dispatch_region;

float getSun(vec2 uv) {
    return length(uv) < 0.009 ? 1.0 : 0.0;
}

vec3 desaturate(vec3 color, float factor) {
    vec3 lum = vec3(0.299, 0.587, 0.114);
    vec3 gray = vec3(dot(lum, color));
    return mix(color, gray, factor);
}

// from: https://www.shadertoy.com/view/XdfXRX
void lensflares(vec2 uv, vec2 pos, out vec3 sunflare, out vec3 lensflare) {
    vec2 main = uv - pos;
    vec2 uvd = uv * (length(uv));

    float ang = atan(main.y, main.x);
    float dist = length(main);
    dist = pow(dist, 0.1);

    float f0 = 1.0 / (length(uv - pos) * 25.0 + 1.0);
    f0 = pow(f0, 2.0);

    f0 = f0 + f0 * (sin((ang + 1.0 / 18.0) * 12.0) * .1 + dist * .1 + .8);

    float f2 =
        max(1.0 / (1.0 + 32.0 * length_squared(uvd + 0.8 * pos)), .0) * 00.25;
    float f22 =
        max(1.0 / (1.0 + 32.0 * length_squared(uvd + 0.85 * pos)), .0) * 00.23;
    float f23 =
        max(1.0 / (1.0 + 32.0 * length_squared(uvd + 0.9 * pos)), .0) * 00.21;

    vec2 uvx = mix(uv, uvd, -0.5);

    float f4 = max(0.01 - pow(length(uvx + 0.4 * pos), 2.4), .0) * 6.0;
    float f42 = max(0.01 - pow(length(uvx + 0.45 * pos), 2.4), .0) * 5.0;
    float f43 = max(0.01 - pow(length(uvx + 0.5 * pos), 2.4), .0) * 3.0;

    uvx = mix(uv, uvd, -.4);

    float f5 = max(0.01 - pow(length(uvx + 0.2 * pos), 5.5), .0) * 2.0;
    float f52 = max(0.01 - pow(length(uvx + 0.4 * pos), 5.5), .0) * 2.0;
    float f53 = max(0.01 - pow(length(uvx + 0.6 * pos), 5.5), .0) * 2.0;

    uvx = mix(uv, uvd, -0.5);

    float f6 = max(0.01 - pow(length(uvx - 0.3 * pos), 1.6), .0) * 6.0;
    float f62 = max(0.01 - pow(length(uvx - 0.325 * pos), 1.6), .0) * 3.0;
    float f63 = max(0.01 - pow(length(uvx - 0.35 * pos), 1.6), .0) * 5.0;

    sunflare = vec3(f0);
    lensflare =
        vec3(f2 + f4 + f5 + f6, f22 + f42 + f52 + f62, f23 + f43 + f53 + f63);

    // seems to already be completely white
    sunflare = desaturate(sunflare, 1.0);

    // Problem: Lens flare has yellow tint
    // Solution desaturate
    // lensflare = desaturate(lensflare, 1.0);
    // Problem: Now chromatic abberation effect is gone
    // Solution: Desaturate only yellow amount
    float yellow = min(lensflare.r, lensflare.g); // amount of yellow
    lensflare.r -= yellow * 0.5;
    lensflare.g -= yellow * 0.5;
}

vec3 anflares(vec2 uv, float threshold, float intensity, float stretch,
    float brightness) {
    threshold = 1.0 - threshold;

    vec3 hdr = vec3(getSun(uv));
    hdr = vec3(floor(threshold + pow(hdr.r, 1.0)));

    float d = intensity;
    float c = intensity * stretch;

    for (float i = c; i > -1.0; i--) {
        float texL = getSun(uv + vec2(i / d, 0.0));
        float texR = getSun(uv - vec2(i / d, 0.0));

        hdr += floor(threshold + pow(max(texL, texR), 4.0)) * (1.0 - i / c);
    }

    return hdr * brightness;
}

vec3 anflares2(vec2 uv, float intensity, float stretch, float brightness) {
    uv.x *= 1.0 / (intensity * stretch);
    uv.y *= 0.5;
    return vec3(smoothstep(0.009, 0.0, length(uv))) * brightness;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Dawn supplies projected solar data explicitly. Only reverse-Z depth and the
// actual color-image dimensions are needed; no engine-private UBO layout.
// ---------------------------------------------------------------------------
void main() {
    ivec2 local_coord = ivec2(gl_GlobalInvocationID.xy);
    ivec2 image_coord = local_coord + ivec2(dispatch_region.rect.xy);
    ivec2 image_size = imageSize(color_image);
    // No always-on artifacts or out-of-bounds depth fetches behind the camera.
    vec2 source_uv = vec2(datablock.data.sun_position_x, datablock.data.sun_position_y);
    if (datablock.data.dir_mult <= 0.0 || any(lessThan(source_uv, vec2(0.0)))
        || any(greaterThanEqual(source_uv, vec2(1.0)))) {
        return;
    }
    // The same solar-disc occlusion applies to every output pixel. Share its
    // nine depth taps per workgroup, rather than repeating them per pixel.
    if (gl_LocalInvocationIndex == 0) {
        ivec2 center = ivec2(source_uv * vec2(image_size));
        float visibility = float(get_raw_depth(center) <= 0.0);
        #ifdef EXPENSIVE_OCCLUSION_TEST
        int spread = max(1, int(datablock.data.occlusion_radius));
        const ivec2 offsets[8] = ivec2[8](
            ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1),
            ivec2(1, 1), ivec2(1, -1), ivec2(-1, 1), ivec2(-1, -1));
        for (int i = 0; i < 8; i++) {
            visibility += float(get_raw_depth(center + offsets[i] * spread) <= 0.0);
        }
        visibility /= 9.0;
        #endif
        source_visibility = visibility;
    }
    barrier();
    // Bounds checks must follow the workgroup barrier, including partial tiles.
    if (any(greaterThanEqual(local_coord, ivec2(dispatch_region.rect.zw)))
        || any(greaterThanEqual(image_coord, image_size))) {
        return;
    }

    vec2 resolution = vec2(image_size);
    vec2 pixel_uv = (vec2(image_coord) + 0.5) / resolution;
    vec2 local_uv = (pixel_uv - source_uv) * resolution / resolution.y;
    float radius = datablock.data.effect_radius;
    float distance_to_sun = length(local_uv);
    float envelope = 1.0 - smoothstep(radius * 0.7, radius, distance_to_sun);
    if (envelope <= 0.0) {
        return;
    }
    vec4 previous_color = imageLoad(color_image, image_coord);
    vec2 uv = pixel_uv - 0.5;
    vec2 sun_pos_norm = vec2(datablock.data.sun_position_x, datablock.data.sun_position_y);
    vec4 sun_color = datablock.data.sun_color;

    vec2 mouse = sun_pos_norm - vec2(0.5);
    mouse.x *= resolution.x / resolution.y;
    uv.x *= resolution.x / resolution.y;

    // ========== God rays ==========
    // EVE's depth-effects approach: march toward the local solar sprite,
    // subtract occluded samples from a faint glow. Stable pixel jitter avoids
    // moving random noise; opaque foreground surfaces receive no glow.
    float godrays = 0.0;
    if (datablock.data.weight > 0.0 && get_raw_depth(image_coord) <= 0.0) {
        int samples = clamp(int(datablock.data.sample_count), 1, 64);
        float jitter = hash(vec2(image_coord));
        float blocked = 0.0;
        float total = 0.0;
        float weight = 1.0;
        for (int i = 0; i < samples; i++) {
            float t = (float(i) + jitter) / float(samples) * datablock.data.density;
            vec2 sample_uv = mix(pixel_uv, sun_pos_norm, t);
            blocked += float(get_raw_depth(ivec2(sample_uv * resolution)) > 0.0) * weight;
            total += weight;
            weight *= datablock.data.decay;
        }
        float transmission = 1.0 - blocked / max(total, 0.0001);
        godrays = transmission * transmission * transmission
            * exp(-3.0 * distance_to_sun / radius) * datablock.data.weight;
    }
    if (source_visibility <= 0.0 && godrays <= 0.0) {
        return;
    }

    // ========== Flares ==========
    vec3 sunflare = vec3(0.0);
    vec3 lensflare = vec3(0.0);

    // Sets sunflare and lensflare
    lensflares(uv * 1.5, mouse * 1.5, sunflare, lensflare);

    #ifndef CHEAP_FLARE
    vec3 anflare = pow(anflares2(uv - mouse, datablock.data.anamorphic_intensity, datablock.data.anamorphic_stretch, datablock.data.anamorphic_brightness), vec3(4.0));
    anflare += smoothstep(0.0025, 1.0, anflare) * 10.0;
    anflare *= smoothstep(0.0, 1.0, anflare);
    #else
    vec3 anflare = pow(anflares(uv - mouse, datablock.data.anamorphic_treshold, datablock.data.anamorphic_intensity, datablock.data.anamorphic_stretch, datablock.data.anamorphic_brightness), vec3(4.0));
    #endif

    // Dawn's sky owns the physical stellar disc. Only add optical artifacts.
    vec3 col = (sunflare + anflare + lensflare)
        * datablock.data.dir_mult * source_visibility * sun_color.rgb * sun_color.a;

    col = 1.0 - exp(-datablock.data.effect_easing * col);
    col *= datablock.data.effect_multiplier;

    // Optical glow has independent strength; do not multiply the entire flare
    // by the shaft mask or create an artificial second solar disc.
    col += godrays * sun_color.rgb * sun_color.a * datablock.data.dir_mult;
    col *= envelope;

    vec4 new_color = vec4(previous_color.rgb + col, previous_color.a);
    imageStore(color_image, image_coord, new_color);
}
