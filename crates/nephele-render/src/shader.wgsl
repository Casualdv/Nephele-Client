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
@group(0) @binding(1) var block_textures: texture_2d_array<f32>;
@group(0) @binding(2) var block_sampler: sampler;
@group(0) @binding(3) var animation_atlas: texture_2d<f32>;
@group(0) @binding(4) var tint_masks: texture_2d_array<f32>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) shade: f32,
    @location(3) layer: u32,
    @location(4) anim: u32,
    @location(5) tint: u32,
    @location(6) light: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) @interpolate(flat) anim: u32,
    @location(4) light: f32,
    @location(5) @interpolate(flat) tint: u32,
    @location(6) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.layer = in.layer;
    out.shade = in.shade;
    out.anim = in.anim;
    out.light = in.light;
    out.tint = in.tint;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // `in.anim` is the animation row, or 0xFFFFFFFF for a static texture.
    // `in.tint` is the packed 0xRRGGBB biome tint; white is the no-op.
    let anim = in.anim;

    // `in.layer` is a packed tile slot: 16 tiles per 64x64 layer, 4x4 inside.
    let local = in.layer % 16u;
    let offset = vec2<f32>(f32(local % 4u), f32(local / 4u));
    let uv = (in.uv + offset) * 0.25;
    let ddx = dpdx(uv);
    let ddy = dpdy(uv);
    var texel = textureSampleGrad(
        block_textures,
        block_sampler,
        uv,
        i32(in.layer / 16u),
        ddx,
        ddy,
    );

    // `.mcmeta` animations live in a 2D strip atlas. The vertex carries the
    // animation row, while the current frame comes from the tick clock.
    if (anim != 0xFFFFFFFFu) {
        let index = anim;
        let frame_info = uniforms.animations[index * 2u];
        let timing = uniforms.animations[index * 2u + 1u];
        let count = max(u32(timing.y), 1u);
        let frame = u32(floor(uniforms.animation.x / max(timing.z, 1.0))) % count;
        let anim_uv = vec2<f32>(
            frame_info.x + f32(frame) * timing.x + in.uv.x * frame_info.z,
            frame_info.y + in.uv.y * frame_info.w,
        );
        texel = textureSampleLevel(animation_atlas, block_sampler, anim_uv, 0.0);
    }

    // Leaves, glass and other cutout textures use alpha testing.
    if (texel.a < 0.5) {
        discard;
    }

    // Biome tint. Only the tiles vanilla tints carry a non-zero mask, so
    // stone, wood and dirt keep their own colour even when they share a face.
    // The mask uses the same packed layout as the block array: `uv` is already
    // packed, and the array layer is the packed slot's layer.
    let mask = textureSampleLevel(
        tint_masks,
        block_sampler,
        uv,
        i32(in.layer / 16u),
        0.0,
    ).a;
    let tint = vec3<f32>(
        f32((in.tint >> 16u) & 255u),
        f32((in.tint >> 8u) & 255u),
        f32(in.tint & 255u),
    ) / 255.0;
    let tinted = texel.rgb * mix(vec3<f32>(1.0), tint, mask);

    let lit = tinted * in.shade * in.light;
    let dist = length(in.world_pos - uniforms.camera.xyz);
    let fog = clamp(
        (dist - uniforms.environment.y)
            / max(uniforms.environment.z - uniforms.environment.y, 0.001),
        0.0,
        1.0,
    );
    let color = mix(lit, uniforms.fog_color.rgb, fog * uniforms.environment.w);
    return vec4<f32>(color, 1.0);
}
