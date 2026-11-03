struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    fog_color: vec4<f32>,
    environment: vec4<f32>,
    animation: vec4<f32>,
    animations: array<vec4<f32>, 64>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct SkyOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: SkyOut;
    out.ndc = positions[index];
    out.clip_position = vec4<f32>(positions[index], 0.0, 1.0);
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn cloud_noise(p: vec2<f32>) -> f32 {
    return value_noise(p) * 0.62
        + value_noise(p * 2.03 + vec2<f32>(17.1, 9.7)) * 0.26
        + value_noise(p * 4.07 + vec2<f32>(3.7, 21.3)) * 0.12;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = uniforms.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let world = far.xyz / far.w;
    let dir = normalize(world - uniforms.camera.xyz);

    let t = fract(uniforms.environment.x / 24000.0);
    let angle = t * 6.2831853;
    let sun = normalize(vec3<f32>(cos(angle), sin(angle), 0.25));
    let moon = -sun;
    let day = smoothstep(-0.10, 0.18, sun.y);
    let sunset = smoothstep(0.0, 0.22, 1.0 - abs(sun.y)) * (1.0 - day * 0.35);

    let night_zenith = vec3<f32>(0.002, 0.005, 0.018);
    let night_horizon = vec3<f32>(0.018, 0.030, 0.075);
    let day_zenith = vec3<f32>(0.22, 0.44, 0.94);
    let day_horizon = vec3<f32>(0.68, 0.83, 1.0);
    let zenith = mix(night_zenith, day_zenith, day);
    var horizon = mix(night_horizon, day_horizon, day);
    horizon = mix(horizon, vec3<f32>(1.0, 0.45, 0.22), sunset * 0.45);

    let up = clamp(dir.y, 0.0, 1.0);
    var sky = mix(horizon, zenith, pow(up, 0.52));
    sky = mix(
        sky,
        uniforms.fog_color.rgb,
        (1.0 - smoothstep(-0.02, 0.20, dir.y)) * 0.32,
    );
    sky = mix(
        sky,
        vec3<f32>(1.0, 0.42, 0.16),
        sunset * 0.55 * (1.0 - smoothstep(0.0, 0.36, dir.y)),
    );

    // Stars are a cheap stable hash in direction space. They only show once
    // the sun is genuinely below the horizon, not during the sunset glow.
    if (sun.y < -0.08 && dir.y > 0.0) {
        let cell = floor(dir * 240.0);
        let h = hash21(cell.xy + cell.z * 0.37);
        if (h > 0.9968) {
            let twinkle = 0.65 + 0.35 * sin(uniforms.environment.x * 0.09 + h * 41.0);
            sky += vec3<f32>(0.85, 0.90, 1.0) * (1.0 - day) * twinkle;
        }
    }

    // Square sun and moon discs, matching vanilla's flat celestial sprites.
    let sun_side = normalize(cross(sun, vec3<f32>(0.0, 0.0, 1.0)));
    let sun_proj = dir - sun * dot(dir, sun);
    let sun_x = dot(sun_proj, sun_side);
    let sun_y = dot(sun_proj, cross(sun, sun_side));
    if (dot(dir, sun) > 0.0 && abs(sun_x) < 0.042 && abs(sun_y) < 0.042) {
        sky = mix(
            vec3<f32>(1.0, 0.96, 0.78),
            vec3<f32>(1.0, 0.55, 0.20),
            sunset * 0.85,
        ) * mix(0.72, 1.0, day);
    }

    let moon_side = normalize(cross(moon, vec3<f32>(0.0, 0.0, 1.0)));
    let moon_proj = dir - moon * dot(dir, moon);
    let moon_x = dot(moon_proj, moon_side);
    let moon_y = dot(moon_proj, cross(moon, moon_side));
    if (dot(dir, moon) > 0.0 && abs(moon_x) < 0.036 && abs(moon_y) < 0.036) {
        sky = mix(sky, vec3<f32>(0.74, 0.78, 0.88), 0.85);
    }

    // Clouds are a procedural layer at y=192. The density threshold and fade
    // keep the edges of the layer from ending in a hard line.
    if (dir.y > 0.015) {
        let distance_to_layer = (192.0 - uniforms.camera.y) / dir.y;
        if (distance_to_layer > 0.0) {
            let wind = uniforms.environment.x * 0.00035;
            let p = (uniforms.camera.xz + dir.xz * distance_to_layer) * 0.006;
            let n = cloud_noise(p + vec2<f32>(wind, wind * 0.35));
            let cover = smoothstep(0.63, 0.80, n)
                * smoothstep(0.015, 0.14, dir.y)
                * (1.0 - smoothstep(700.0, 1400.0, distance_to_layer));
            let cloud = mix(
                vec3<f32>(0.045, 0.055, 0.095),
                vec3<f32>(0.96, 0.98, 1.0),
                day,
            );
            sky = mix(sky, cloud, cover * 0.72);
        }
    }

    let glow = pow(max(dot(dir, sun), 0.0), 320.0) * 0.10 * day;
    sky += vec3<f32>(1.0, 0.78, 0.46) * glow;
    return vec4<f32>(sky, 1.0);
}
