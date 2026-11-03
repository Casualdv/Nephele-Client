struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var block_textures: texture_2d_array<f32>;
@group(0) @binding(2) var block_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) shade: f32,
    @location(3) layer: u32,
    @location(4) light: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) light: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.layer = in.layer;
    out.shade = in.shade;
    out.light = in.light;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // `in.layer` is a packed tile slot: 16 tiles per 64×64 layer, 4×4 inside.
    let local = in.layer % 16u;
    let offset = vec2<f32>(f32(local % 4u), f32(local / 4u));
    let uv = (in.uv + offset) * 0.25;
    let texel = textureSample(block_textures, block_sampler, uv, in.layer / 16u);
    // Leaves, glass and other cutout textures use alpha testing.
    if (texel.a < 0.5) {
        discard;
    }
    return vec4<f32>(texel.rgb * in.shade * in.light, 1.0);
}
