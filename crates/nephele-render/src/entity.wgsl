// Entity (player / mob) rendering: a 2D skin atlas sampled with per-part UVs.

struct Uniforms {
    view_proj: mat4x4<f32>,
    world_light: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var skin_atlas: texture_2d<f32>;
@group(0) @binding(2) var skin_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) shade: f32,
    @location(3) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) shade: f32,
    @location(2) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.shade = in.shade;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(skin_atlas, skin_sampler, in.uv);
    // Fully transparent pixels on a skin are the "no part here" marker.
    if (texel.a < 0.1) {
        discard;
    }
    let rgb = texel.rgb * in.shade * in.color.rgb * uniforms.world_light;
    return vec4<f32>(rgb, texel.a * in.color.a);
}
