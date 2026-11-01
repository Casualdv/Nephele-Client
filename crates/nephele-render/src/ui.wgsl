struct UiUniforms {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: UiUniforms;
@group(0) @binding(1) var ui_atlas: texture_2d<f32>;
@group(0) @binding(2) var block_textures: texture_2d_array<f32>;
@group(0) @binding(3) var ui_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) layer: u32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) layer: u32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Pixels -> clip space, origin top-left.
    let ndc = vec2<f32>(
        in.position.x / uniforms.screen.x * 2.0 - 1.0,
        1.0 - in.position.y / uniforms.screen.y * 2.0,
    );
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.layer = in.layer;
    return out;
}

@fragment
fn fs_sprite(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(ui_atlas, ui_sampler, in.uv);
    return vec4<f32>(texel.rgb * in.color.rgb, texel.a * in.color.a);
}

@fragment
fn fs_block(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(block_textures, ui_sampler, in.uv, in.layer);
    if (texel.a < 0.1) {
        discard;
    }
    return vec4<f32>(texel.rgb * in.color.rgb, texel.a * in.color.a);
}
