use nephele_assets::{
    face_textures, is_transparent, item_texture_name, TextureArray, MISSING_TEXTURE,
};
use nephele_world::shapes::{
    block_shape, door_shape, is_door, is_full_cube, is_pane, pane_shape, PaneConnections, PaneShape,
};
use nephele_world::{BlockState, ChunkColumn, ChunkCoord, World};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub shade: f32,
    /// Texture-array layer for this face.
    pub layer: u32,
    /// Optional index into the renderer's `.mcmeta` animation table.
    pub anim: u32,
    /// Combined block/sky illumination with ambient occlusion, in 0..=1.
    pub light: f32,
}

/// Sentinel for vertices that use a static texture-array layer.
pub const NO_ANIMATION: u32 = u32::MAX;

impl Vertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Float32,
        3 => Uint32,
        4 => Uint32,
        5 => Float32
    ];

    pub const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn append(&mut self, other: MeshData) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }
}

/// Brightness multiplier per face, giving cheap directional shading without
/// any lighting pass (the same trick vanilla uses for its block models).
struct FaceDef {
    normal: [i32; 3],
    corners: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    shade: f32,
    /// Index into a block's `[top, bottom, side]` texture layers.
    slot: usize,
}

const FACES: [FaceDef; 6] = [
    // +Y
    FaceDef {
        normal: [0, 1, 0],
        corners: [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        shade: 1.0,
        slot: 0,
    },
    // -Y
    FaceDef {
        normal: [0, -1, 0],
        corners: [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        uvs: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        shade: 0.5,
        slot: 1,
    },
    // -Z
    FaceDef {
        normal: [0, 0, -1],
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        uvs: [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        shade: 0.8,
        slot: 2,
    },
    // +Z
    FaceDef {
        normal: [0, 0, 1],
        corners: [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        uvs: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        shade: 0.8,
        slot: 2,
    },
    // +X
    FaceDef {
        normal: [1, 0, 0],
        corners: [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
        uvs: [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        shade: 0.65,
        slot: 2,
    },
    // -X
    FaceDef {
        normal: [-1, 0, 0],
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
        uvs: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        shade: 0.65,
        slot: 2,
    },
];

/// Crack overlay for the block currently being mined: a cube grown by 3 mm
/// around the block, textured with `destroy_stage_<stage>`.
pub fn build_break_overlay(x: i32, y: i32, z: i32, layer: u32) -> MeshData {
    const GROW: f32 = 0.003;
    let place = |value: f32, origin: i32| -> f32 {
        if value == 0.0 {
            origin as f32 - GROW
        } else {
            origin as f32 + 1.0 + GROW
        }
    };

    let mut mesh = MeshData::default();
    for face in &FACES {
        let base = mesh.vertices.len() as u32;
        for (corner, uv) in face.corners.iter().zip(face.uvs.iter()) {
            mesh.vertices.push(Vertex {
                position: [
                    place(corner[0], x),
                    place(corner[1], y),
                    place(corner[2], z),
                ],
                uv: *uv,
                // Vanilla renders the crack overlay unshaded.
                shade: 1.0,
                layer,
                anim: NO_ANIMATION,
                light: 1.0,
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

/// Texture name for a crack stage (`destroy_stage_0` .. `destroy_stage_9`).
pub fn break_overlay_texture(stage: u8) -> &'static str {
    const NAMES: [&str; 10] = [
        "destroy_stage_0",
        "destroy_stage_1",
        "destroy_stage_2",
        "destroy_stage_3",
        "destroy_stage_4",
        "destroy_stage_5",
        "destroy_stage_6",
        "destroy_stage_7",
        "destroy_stage_8",
        "destroy_stage_9",
    ];
    NAMES[(stage as usize).min(9)]
}

const PLANE_LEN: usize = 16 * 256;
const LIGHT_VOLUME: usize = 16 * 16 * 256;
const CORNER_LEN: usize = 256;

/// Everything needed to mesh one chunk column *without* touching the world:
/// the column's own blocks plus the four neighbouring 16×256 faces used for
/// border culling.
///
/// Owning a snapshot (instead of borrowing `World`) is what lets meshing run on
/// a worker thread while the main thread keeps streaming chunks in.
#[derive(Debug, Clone)]
pub struct ColumnMeshJob {
    pub key: (i32, i32),
    /// One entry per 16-block section; `None` when the section is all air.
    pub sections: Vec<Option<Vec<u16>>>,
    /// Blocks of the neighbour at local x = 15, indexed `y * 16 + z`.
    pub west: Vec<u16>,
    /// Blocks of the neighbour at local x = 0, indexed `y * 16 + z`.
    pub east: Vec<u16>,
    /// Blocks of the neighbour at local z = 15, indexed `y * 16 + x`.
    pub north: Vec<u16>,
    /// Blocks of the neighbour at local z = 0, indexed `y * 16 + x`.
    pub south: Vec<u16>,
    /// Effective 0..15 light for each local block cell, indexed `y * 256 + z * 16 + x`.
    pub light_levels: Vec<u8>,
    /// Whether this dimension has skylight. Used above the build height and for
    /// direct-sky fallback in locally generated chunks without light arrays.
    pub has_skylight: bool,
    /// Four corner columns, NW/NE/SW/SE, used by smooth-light samples at chunk corners.
    pub corners: [Vec<u16>; 4],
    pub corner_lights: [Vec<u8>; 4],
    pub west_lights: Vec<u8>,
    pub east_lights: Vec<u8>,
    pub north_lights: Vec<u8>,
    pub south_lights: Vec<u8>,
}

impl ColumnMeshJob {
    pub fn from_world(key: (i32, i32), world: &World) -> Self {
        let mut sections = vec![None; nephele_world::SECTION_COUNT];
        let column = world.get(ChunkCoord::new(key.0, key.1));
        if let Some(column) = column {
            for (index, section) in column.iter_sections() {
                if section.non_air_count() > 0 {
                    sections[index] = Some(section.blocks().to_vec());
                }
            }
        }
        let has_skylight = world.has_skylight();
        Self {
            key,
            sections,
            west: side_plane(world, key.0 - 1, key.1, Axis::X, 15),
            east: side_plane(world, key.0 + 1, key.1, Axis::X, 0),
            north: side_plane(world, key.0, key.1 - 1, Axis::Z, 15),
            south: side_plane(world, key.0, key.1 + 1, Axis::Z, 0),
            light_levels: light_volume(column, has_skylight),
            has_skylight,
            corners: [
                corner_column(world, key.0 - 1, key.1 - 1, 15, 15),
                corner_column(world, key.0 + 1, key.1 - 1, 0, 15),
                corner_column(world, key.0 - 1, key.1 + 1, 15, 0),
                corner_column(world, key.0 + 1, key.1 + 1, 0, 0),
            ],
            corner_lights: [
                light_corner_column(world, key.0 - 1, key.1 - 1, 15, 15, has_skylight),
                light_corner_column(world, key.0 + 1, key.1 - 1, 0, 15, has_skylight),
                light_corner_column(world, key.0 - 1, key.1 + 1, 15, 0, has_skylight),
                light_corner_column(world, key.0 + 1, key.1 + 1, 0, 0, has_skylight),
            ],
            west_lights: light_side_plane(world, key.0 - 1, key.1, Axis::X, 15, has_skylight),
            east_lights: light_side_plane(world, key.0 + 1, key.1, Axis::X, 0, has_skylight),
            north_lights: light_side_plane(world, key.0, key.1 - 1, Axis::Z, 15, has_skylight),
            south_lights: light_side_plane(world, key.0, key.1 + 1, Axis::Z, 0, has_skylight),
        }
    }

    pub fn from_column(column: &ChunkColumn, world: &World) -> Self {
        let mut job = Self::from_world((column.x, column.z), world);
        for (index, section) in column.iter_sections() {
            if section.non_air_count() > 0 {
                job.sections[index] = Some(section.blocks().to_vec());
            }
        }
        job.light_levels = light_volume(Some(column), job.has_skylight);
        job
    }

    /// Block lookup in column-local coordinates, where the border neighbours
    /// (x/z of -1 or 16) resolve through the side planes.
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> u16 {
        if !(0..256).contains(&y) {
            return 0;
        }
        let yi = y as usize;
        let inside_x = (0..16).contains(&x);
        let inside_z = (0..16).contains(&z);

        if inside_x && inside_z {
            return match &self.sections[yi >> 4] {
                Some(blocks) => blocks[((yi & 15) << 8) | ((z as usize) << 4) | (x as usize)],
                None => 0,
            };
        }
        if !inside_x && !inside_z {
            return self.corners[corner_index(x, z)][yi];
        }
        // Exactly one axis stepped out of the column, or a diagonal corner.
        let row = yi * 16;
        if !inside_x {
            return if x < 0 {
                self.west[row + z as usize]
            } else {
                self.east[row + z as usize]
            };
        }
        if !inside_z {
            return if z < 0 {
                self.north[row + x as usize]
            } else {
                self.south[row + x as usize]
            };
        }
        0
    }

    /// Effective block/sky light in 0..=15. Out-of-column reads use the same
    /// border snapshot as `block`, including diagonal corner columns.
    #[inline]
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        if y < 0 {
            return 0;
        }
        if y >= 256 {
            return if self.has_skylight { 15 } else { 0 };
        }
        let yi = y as usize;
        let inside_x = (0..16).contains(&x);
        let inside_z = (0..16).contains(&z);

        if inside_x && inside_z {
            return self.light_levels[yi * 256 + z as usize * 16 + x as usize];
        }
        if !inside_x && !inside_z {
            return self.corner_lights[corner_index(x, z)][yi];
        }

        let row = yi * 16;
        if !inside_x {
            return if x < 0 {
                self.west_lights[row + z as usize]
            } else {
                self.east_lights[row + z as usize]
            };
        }
        if !inside_z {
            return if z < 0 {
                self.north_lights[row + x as usize]
            } else {
                self.south_lights[row + x as usize]
            };
        }
        0
    }
}

enum Axis {
    X,
    Z,
}

/// Copy one 16×256 face off a neighbouring column. Reads the section data
/// directly — 4096 array reads instead of 4096 hash-map lookups.
fn side_plane(world: &World, cx: i32, cz: i32, axis: Axis, local: usize) -> Vec<u16> {
    let mut out = vec![0u16; PLANE_LEN];
    let Some(column) = world.get(ChunkCoord::new(cx, cz)) else {
        return out;
    };
    for (index, section) in column.iter_sections() {
        let blocks = section.blocks();
        for ly in 0..16 {
            // `out` is indexed `y * 16 + t`, with `y` absolute.
            let row = (index * 16 + ly) * 16;
            for t in 0..16 {
                let value = match axis {
                    Axis::X => blocks[nephele_world::section_index(local, ly, t)],
                    Axis::Z => blocks[nephele_world::section_index(t, ly, local)],
                };
                out[row + t] = value;
            }
        }
    }
    out
}

fn corner_index(x: i32, z: i32) -> usize {
    match (x < 0, z < 0) {
        (true, true) => 0,
        (false, true) => 1,
        (true, false) => 2,
        (false, false) => 3,
    }
}

fn corner_column(world: &World, cx: i32, cz: i32, x: usize, z: usize) -> Vec<u16> {
    let mut out = vec![0; CORNER_LEN];
    if let Some(column) = world.get(ChunkCoord::new(cx, cz)) {
        for (y, raw) in out.iter_mut().enumerate() {
            *raw = column.get_block(x, y, z).raw();
        }
    }
    out
}

fn light_corner_column(
    world: &World,
    cx: i32,
    cz: i32,
    x: usize,
    z: usize,
    has_skylight: bool,
) -> Vec<u8> {
    trace_light_column(world.get(ChunkCoord::new(cx, cz)), x, z, has_skylight).to_vec()
}

fn light_side_plane(
    world: &World,
    cx: i32,
    cz: i32,
    axis: Axis,
    local: usize,
    has_skylight: bool,
) -> Vec<u8> {
    let column = world.get(ChunkCoord::new(cx, cz));
    let mut out = vec![0; PLANE_LEN];
    for t in 0..16 {
        let (x, z) = match axis {
            Axis::X => (local, t),
            Axis::Z => (t, local),
        };
        let trace = trace_light_column(column, x, z, has_skylight);
        for (y, light) in trace.into_iter().enumerate() {
            out[y * 16 + t] = light;
        }
    }
    out
}

fn light_volume(column: Option<&ChunkColumn>, has_skylight: bool) -> Vec<u8> {
    let Some(column) = column else {
        return vec![if has_skylight { 15 } else { 0 }; LIGHT_VOLUME];
    };
    let mut out = vec![0; LIGHT_VOLUME];
    for z in 0..16 {
        for x in 0..16 {
            let trace = trace_light_column(Some(column), x, z, has_skylight);
            for (y, light) in trace.into_iter().enumerate() {
                out[y * 256 + z * 16 + x] = light;
            }
        }
    }
    out
}

/// Read decoded light when present. A vertical direct-sky trace fills missing
/// light arrays in locally generated chunks; it deliberately stops at opaque
/// full cubes, so caves are not made bright by the absence of a light engine.
fn trace_light_column(
    column: Option<&ChunkColumn>,
    x: usize,
    z: usize,
    has_skylight: bool,
) -> [u8; 256] {
    let mut out = [0; 256];
    let mut sky_visible = has_skylight;

    for y in (0..256).rev() {
        let state = column
            .map(|column| column.get_block(x, y, z))
            .unwrap_or(BlockState::AIR);
        let section = column.and_then(|column| column.section(y >> 4));
        let block_light = section
            .map(|section| section.block_light_at(x, y & 15, z))
            .unwrap_or(0);
        let sky_light = section
            .and_then(|section| section.sky_light_at(x, y & 15, z))
            .unwrap_or(if sky_visible && !blocks_direct_sky(state) {
                15
            } else {
                0
            });
        out[y] = block_light.max(sky_light);

        if blocks_direct_sky(state) {
            sky_visible = false;
        }
    }
    out
}

fn blocks_direct_sky(state: BlockState) -> bool {
    !state.is_air()
        && !is_transparent(state.id())
        && is_full_cube(block_shape(state.id(), state.meta()))
}

/// Mesh a chunk column from an owned snapshot. Pure computation — no world or
/// GPU access — so it is safe to run on any thread.
pub fn mesh_column(job: &ColumnMeshJob, atlas: &TextureArray) -> MeshData {
    let mut mesh = MeshData::default();
    // Block states repeat heavily, so resolve the texture layers once per
    // distinct state instead of three hash lookups per block.
    type ResolvedTextures = ([u32; 3], [u32; 3]);
    let mut resolved: std::collections::HashMap<u16, Option<ResolvedTextures>> =
        std::collections::HashMap::with_capacity(64);

    let base_x = job.key.0 * 16;
    let base_z = job.key.1 * 16;

    for (section_index, section) in job.sections.iter().enumerate() {
        let Some(blocks) = section else {
            continue;
        };
        let section_y = section_index as i32 * 16;

        for (index, raw) in blocks.iter().copied().enumerate() {
            if raw == 0 {
                continue;
            }
            let layers = match resolved.entry(raw) {
                std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let textures = face_textures(BlockState::from_raw(raw)).map(|textures| {
                        let layers = [
                            atlas.layer_or_missing(textures.top),
                            atlas.layer_or_missing(textures.bottom),
                            atlas.layer_or_missing(textures.side),
                        ];
                        let anims = [
                            atlas.animation(textures.top).unwrap_or(NO_ANIMATION),
                            atlas.animation(textures.bottom).unwrap_or(NO_ANIMATION),
                            atlas.animation(textures.side).unwrap_or(NO_ANIMATION),
                        ];
                        (layers, anims)
                    });
                    *entry.insert(textures)
                }
            };
            let Some((layers, anims)) = layers else {
                continue;
            };

            let local_x = (index & 15) as i32;
            let local_z = ((index >> 4) & 15) as i32;
            let local_y = ((index >> 8) & 15) as i32;
            let y = section_y + local_y;
            let id = raw >> 4;

            // Plants are two crossed quads, not a cube. Rendering them as
            // cubes is what makes flowers and grass look like solid boxes.
            if is_cross_plant(id) {
                let layer = layers[2];
                let anim = anims[2];
                let bx = (base_x + local_x) as f32;
                let by = y as f32;
                let bz = (base_z + local_z) as f32;
                let (lo, hi) = (0.1f32, 0.9f32);
                let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
                let light = job.light(local_x, y + 1, local_z) as f32 / 15.0;

                let quads = [
                    [
                        [bx + lo, by, bz + lo],
                        [bx + hi, by, bz + hi],
                        [bx + hi, by + 1.0, bz + hi],
                        [bx + lo, by + 1.0, bz + lo],
                    ],
                    [
                        [bx + lo, by, bz + hi],
                        [bx + hi, by, bz + lo],
                        [bx + hi, by + 1.0, bz + lo],
                        [bx + lo, by + 1.0, bz + hi],
                    ],
                ];

                for quad in quads {
                    // Both windings, so the plant is visible from either side.
                    for order in [[0usize, 1, 2, 3], [3, 2, 1, 0]] {
                        let base = mesh.vertices.len() as u32;
                        for index in order {
                            mesh.vertices.push(Vertex {
                                position: quad[index],
                                uv: uvs[index],
                                shade: 1.0,
                                layer,
                                anim,
                                light,
                            });
                        }
                        mesh.indices.extend_from_slice(&[
                            base,
                            base + 1,
                            base + 2,
                            base,
                            base + 2,
                            base + 3,
                        ]);
                    }
                }
                continue;
            }
            let meta = (raw & 0x0f) as u8;
            let pane_boxes = is_pane(id).then(|| pane_shape_for_block(job, local_x, y, local_z));
            let shape = if let Some(pane) = pane_boxes.as_ref() {
                pane.as_slice()
            } else if is_door(id) {
                door_shape_for_block(job, id, meta, local_x, y, local_z)
            } else {
                block_shape(id, meta)
            };
            let origin = [
                (base_x + local_x) as f32,
                y as f32,
                (base_z + local_z) as f32,
            ];

            if is_full_cube(shape) {
                // Keep the common cube path on unit-cube coordinates. A whole
                // face can be culled only when its neighbour also fills a cube.
                for face in &FACES {
                    if should_cull_full_cube_face(job, raw, id, local_x, y, local_z, face) {
                        continue;
                    }

                    let positions = std::array::from_fn(|index| {
                        let corner = face.corners[index];
                        [
                            origin[0] + corner[0],
                            origin[1] + corner[1],
                            origin[2] + corner[2],
                        ]
                    });
                    let lights = face_vertex_lights(job, local_x, y, local_z, face);
                    append_quad(
                        &mut mesh,
                        face,
                        layers[face.slot],
                        anims[face.slot],
                        positions,
                        lights,
                    );
                }
            } else {
                // Shape boxes use 0..16 block pixels; convert them to block
                // fractions, then scale each unit-cube face corner into the box.
                for shape_box in shape {
                    let min = shape_box.min.map(|value| value / 16.0);
                    let max = shape_box.max.map(|value| value / 16.0);
                    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];

                    for face in &FACES {
                        let positions = std::array::from_fn(|index| {
                            let corner = face.corners[index];
                            [
                                origin[0] + min[0] + corner[0] * size[0],
                                origin[1] + min[1] + corner[1] * size[1],
                                origin[2] + min[2] + corner[2] * size[2],
                            ]
                        });
                        let lights = face_vertex_lights(job, local_x, y, local_z, face);
                        append_quad(
                            &mut mesh,
                            face,
                            layers[face.slot],
                            anims[face.slot],
                            positions,
                            lights,
                        );
                    }
                }
            }
        }
    }

    mesh
}

fn pane_shape_for_block(job: &ColumnMeshJob, x: i32, y: i32, z: i32) -> PaneShape {
    pane_shape(PaneConnections {
        west: pane_connects(job.block(x - 1, y, z)),
        east: pane_connects(job.block(x + 1, y, z)),
        north: pane_connects(job.block(x, y, z - 1)),
        south: pane_connects(job.block(x, y, z + 1)),
    })
}

fn pane_connects(raw: u16) -> bool {
    let id = raw >> 4;
    is_pane(id) || is_full_cube(block_shape(id, (raw & 0x0f) as u8))
}

fn door_shape_for_block(
    job: &ColumnMeshJob,
    id: u16,
    meta: u8,
    x: i32,
    y: i32,
    z: i32,
) -> &'static [nephele_world::shapes::ShapeBox] {
    let lower_y = if meta & 8 != 0 { y - 1 } else { y };
    let lower = job.block(x, lower_y, z);
    let upper = job.block(x, lower_y + 1, z);
    let lower_meta = if lower >> 4 == id && lower & 8 == 0 {
        (lower & 0x0f) as u8
    } else if meta & 8 == 0 {
        meta
    } else {
        0
    };
    let upper_meta = if upper >> 4 == id && upper & 8 != 0 {
        (upper & 0x0f) as u8
    } else if meta & 8 != 0 {
        meta
    } else {
        0
    };
    door_shape(lower_meta, upper_meta)
}

fn face_vertex_lights(job: &ColumnMeshJob, x: i32, y: i32, z: i32, face: &FaceDef) -> [f32; 4] {
    std::array::from_fn(|corner| vertex_light(job, [x, y, z], face, corner))
}

fn vertex_light(job: &ColumnMeshJob, block: [i32; 3], face: &FaceDef, corner_index: usize) -> f32 {
    let corner = face.corners[corner_index];
    let mut tangent_axes = [0usize; 2];
    let mut tangent_count = 0;
    for axis in 0..3 {
        if face.normal[axis] == 0 {
            tangent_axes[tangent_count] = axis;
            tangent_count += 1;
        }
    }

    let mut side_a = [0; 3];
    let mut side_b = [0; 3];
    side_a[tangent_axes[0]] = if corner[tangent_axes[0]] < 0.5 { -1 } else { 1 };
    side_b[tangent_axes[1]] = if corner[tangent_axes[1]] < 0.5 { -1 } else { 1 };
    let diagonal = [
        side_a[0] + side_b[0],
        side_a[1] + side_b[1],
        side_a[2] + side_b[2],
    ];
    let outside = [
        block[0] + face.normal[0],
        block[1] + face.normal[1],
        block[2] + face.normal[2],
    ];
    let light_samples = [
        outside,
        [
            outside[0] + side_a[0],
            outside[1] + side_a[1],
            outside[2] + side_a[2],
        ],
        [
            outside[0] + side_b[0],
            outside[1] + side_b[1],
            outside[2] + side_b[2],
        ],
        [
            outside[0] + diagonal[0],
            outside[1] + diagonal[1],
            outside[2] + diagonal[2],
        ],
    ];
    let light_sum: u16 = light_samples
        .iter()
        .map(|sample| job.light(sample[0], sample[1], sample[2]) as u16)
        .sum();
    let face_light = light_sum as f32 / (4.0 * 15.0);

    let ao_side_a = [
        block[0] + side_a[0],
        block[1] + side_a[1],
        block[2] + side_a[2],
    ];
    let ao_side_b = [
        block[0] + side_b[0],
        block[1] + side_b[1],
        block[2] + side_b[2],
    ];
    let ao_corner = [
        block[0] + diagonal[0],
        block[1] + diagonal[1],
        block[2] + diagonal[2],
    ];
    let side_a_occludes = is_ao_occluder(job.block(ao_side_a[0], ao_side_a[1], ao_side_a[2]));
    let side_b_occludes = is_ao_occluder(job.block(ao_side_b[0], ao_side_b[1], ao_side_b[2]));
    let corner_occludes = is_ao_occluder(job.block(ao_corner[0], ao_corner[1], ao_corner[2]));
    let occluders =
        u8::from(side_a_occludes) + u8::from(side_b_occludes) + u8::from(corner_occludes);
    let ao = if side_a_occludes && side_b_occludes {
        0.55
    } else {
        1.0 - f32::from(occluders) * 0.15
    };

    (face_light * ao).clamp(0.0, 1.0)
}

fn is_ao_occluder(raw: u16) -> bool {
    let id = raw >> 4;
    id != 0 && !is_transparent(id) && is_full_cube(block_shape(id, (raw & 0x0f) as u8))
}

/// Preserve the existing opacity rules, but only remove a complete shared face
/// when both block states are full cubes. A partial shape may cover just part of
/// the face, so treating it as a whole-face occluder would create holes.
fn should_cull_full_cube_face(
    job: &ColumnMeshJob,
    raw: u16,
    id: u16,
    local_x: i32,
    y: i32,
    local_z: i32,
    face: &FaceDef,
) -> bool {
    let neighbour = job.block(
        local_x + face.normal[0],
        y + face.normal[1],
        local_z + face.normal[2],
    );
    let neighbour_id = neighbour >> 4;
    let neighbour_meta = (neighbour & 0x0f) as u8;
    if !is_full_cube(block_shape(neighbour_id, neighbour_meta)) {
        return false;
    }

    if !is_transparent(neighbour_id) {
        return true;
    }
    // Two of the same see-through block still hide their shared face.
    neighbour == raw && is_transparent(id)
}

fn append_quad(
    mesh: &mut MeshData,
    face: &FaceDef,
    layer: u32,
    anim: u32,
    positions: [[f32; 3]; 4],
    lights: [f32; 4],
) {
    let base = mesh.vertices.len() as u32;
    for ((position, uv), light) in positions
        .into_iter()
        .zip(face.uvs.iter().copied())
        .zip(lights)
    {
        mesh.vertices.push(Vertex {
            position,
            uv,
            shade: face.shade,
            layer,
            anim,
            light,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Blocks drawn as two crossed quads rather than a cube.
///
/// Saplings, grass, flowers, mushrooms, crops, sugar cane and the double
/// plants. Everything else keeps the cube path.
fn is_cross_plant(id: u16) -> bool {
    matches!(
        id,
        6 | 31 | 32 | 37 | 38 | 39 | 40 | 59 | 83 | 141 | 142 | 175
    )
}
/// Mesh one chunk column straight from the world.
pub fn build_column_mesh(column: &ChunkColumn, world: &World, atlas: &TextureArray) -> MeshData {
    let job = ColumnMeshJob::from_column(column, world);
    mesh_column(&job, atlas)
}

/// Mesh every loaded column in the world into one buffer (used by the
/// synthetic `view` mode).
pub fn build_world_mesh(world: &World, atlas: &TextureArray) -> MeshData {
    let mut mesh = MeshData::default();
    for (coord, _) in world.iter() {
        let job = ColumnMeshJob::from_world((coord.x, coord.z), world);
        mesh.append(mesh_column(&job, atlas));
    }
    mesh
}

/// One dropped item to draw this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemDraw {
    /// Feet position in world space, i.e. the block corner the item sits on.
    pub position: [f32; 3],
    /// Item id. Block items use block textures; mapped non-block items use item textures.
    pub item: i16,
    pub meta: i16,
}

/// Edge length of a dropped item, in blocks. Vanilla's is 0.25 tip to tip.
const ITEM_SIZE: f32 = 0.25;
/// How far above the block below it an item rests.
const ITEM_HOVER: f32 = 0.1;
/// Amplitude of the idle bob.
const ITEM_BOB: f32 = 0.04;

/// Dropped items: a small spinning, bobbing cube per stack.
///
/// Block items use their block faces. Mapped non-block items use their prepared
/// item texture on each face; unmapped ids use the generated missing tile.
pub fn build_item_mesh(items: &[ItemDraw], atlas: &TextureArray, time: f32) -> MeshData {
    let mut mesh = MeshData::default();
    for item in items {
        let layers = if let Some(texture) = item_texture_name(item.item, item.meta) {
            [atlas.layer_or_missing(texture); 3]
        } else {
            let state = BlockState::new(item.item.max(0) as u16, item.meta.clamp(0, 15) as u8);
            match face_textures(state) {
                Some(faces) => [
                    atlas.layer_or_missing(faces.top),
                    atlas.layer_or_missing(faces.bottom),
                    atlas.layer_or_missing(faces.side),
                ],
                None => [atlas.layer_or_missing(MISSING_TEXTURE); 3],
            }
        };

        // A per-item phase so a pile of drops doesn't spin in lockstep.
        let phase = item.position[0] * 1.7 + item.position[2] * 0.9;
        let (sin, cos) = (time * 1.2 + phase).sin_cos();
        let bob = (time * 2.0 + phase).sin() * ITEM_BOB;
        let centre = [
            item.position[0] + 0.5,
            item.position[1] + ITEM_HOVER + ITEM_SIZE * 0.5 + bob,
            item.position[2] + 0.5,
        ];

        for face in &FACES {
            let base = mesh.vertices.len() as u32;
            for (corner, uv) in face.corners.iter().zip(face.uvs.iter()) {
                // Centre the cube on the origin, shrink it, then spin about Y.
                let local = [
                    (corner[0] - 0.5) * ITEM_SIZE,
                    (corner[1] - 0.5) * ITEM_SIZE,
                    (corner[2] - 0.5) * ITEM_SIZE,
                ];
                mesh.vertices.push(Vertex {
                    position: [
                        centre[0] + local[0] * cos - local[2] * sin,
                        centre[1] + local[1],
                        centre[2] + local[0] * sin + local[2] * cos,
                    ],
                    uv: *uv,
                    shade: face.shade,
                    layer: layers[face.slot],
                    anim: NO_ANIMATION,
                    light: 1.0,
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    mesh
}

/// A block held in the first-person hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandItem {
    /// Item id; block items use that block's textures.
    pub item: i16,
    pub meta: i16,
    /// World-space centre of the cube.
    pub centre: [f32; 3],
    /// World-space axes; `right x up` must equal `forward`.
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    /// Edge length in blocks.
    pub size: f32,
}

/// Build one held block cube oriented along the hand's axes.
pub fn build_hand_item(item: &HandItem, atlas: &TextureArray) -> MeshData {
    let mut mesh = MeshData::default();
    let state = BlockState::new(item.item.max(0) as u16, item.meta.clamp(0, 15) as u8);
    let layers = match face_textures(state) {
        Some(faces) => [
            atlas.layer_or_missing(faces.top),
            atlas.layer_or_missing(faces.bottom),
            atlas.layer_or_missing(faces.side),
        ],
        None => [atlas.layer_or_missing(MISSING_TEXTURE); 3],
    };

    let right = glam::Vec3::from(item.right);
    let up = glam::Vec3::from(item.up);
    let forward = glam::Vec3::from(item.forward);
    let centre = glam::Vec3::from(item.centre);

    for face in &FACES {
        let base = mesh.vertices.len() as u32;
        for (corner, uv) in face.corners.iter().zip(face.uvs.iter()) {
            let local = right * ((corner[0] - 0.5) * item.size)
                + up * ((corner[1] - 0.5) * item.size)
                + forward * ((corner[2] - 0.5) * item.size);
            mesh.vertices.push(Vertex {
                position: (centre + local).to_array(),
                uv: *uv,
                shade: face.shade,
                layer: layers[face.slot],
                anim: NO_ANIMATION,
                light: 1.0,
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use nephele_assets::TextureArray;
    use nephele_world::ChunkColumn;

    fn atlas() -> TextureArray {
        TextureArray::from_tiles(vec![
            (
                "stone",
                RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255])),
            ),
            (
                "item:stick",
                RgbaImage::from_pixel(16, 16, image::Rgba([200, 150, 100, 255])),
            ),
        ])
    }

    fn world_with(column: ChunkColumn) -> World {
        let mut world = World::new(0);
        world.insert(column);
        world
    }

    fn set_light_nibble(data: &mut [u8], x: usize, y: usize, z: usize, level: u8) {
        let index = nephele_world::section_index(x, y, z);
        let byte = &mut data[index / 2];
        if index & 1 == 0 {
            *byte = (*byte & 0xF0) | (level & 0x0F);
        } else {
            *byte = (*byte & 0x0F) | ((level & 0x0F) << 4);
        }
    }

    fn set_block_light(column: &mut ChunkColumn, x: usize, y: usize, z: usize, level: u8) {
        let section_index = y >> 4;
        let mut data = column
            .section(section_index)
            .unwrap()
            .block_light()
            .to_vec();
        set_light_nibble(&mut data, x, y & 15, z, level);
        column
            .section_mut(section_index)
            .unwrap()
            .set_block_light(&data);
    }

    #[test]
    fn full_cube_emits_six_faces() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, BlockState::STONE);
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
    }

    fn single_block_mesh(state: BlockState) -> MeshData {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, state);
        let world = world_with(column.clone());
        build_column_mesh(&column, &world, &atlas())
    }

    fn bounds(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in vertices {
            for (axis, value) in vertex.position.iter().enumerate() {
                min[axis] = min[axis].min(*value);
                max[axis] = max[axis].max(*value);
            }
        }
        (min, max)
    }

    #[test]
    fn slab_mesh_uses_bottom_or_top_half_height() {
        let bottom = single_block_mesh(BlockState::new(44, 0));
        assert_eq!(bottom.vertices.len(), 24);
        assert_eq!(
            bounds(&bottom.vertices),
            ([0.0, 40.0, 0.0], [1.0, 40.5, 1.0])
        );

        let top = single_block_mesh(BlockState::new(44, 8));
        assert_eq!(top.vertices.len(), 24);
        assert_eq!(bounds(&top.vertices), ([0.0, 40.5, 0.0], [1.0, 41.0, 1.0]));
    }

    #[test]
    fn stair_mesh_emits_the_base_and_oriented_step_boxes() {
        let mesh = single_block_mesh(BlockState::new(53, 0));
        assert_eq!(mesh.vertices.len(), 48);
        assert_eq!(
            bounds(&mesh.vertices[24..]),
            ([0.5, 40.5, 0.0], [1.0, 41.0, 1.0])
        );
    }

    #[test]
    fn fence_mesh_uses_the_narrow_post() {
        let mesh = single_block_mesh(BlockState::new(85, 0));
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(
            bounds(&mesh.vertices),
            (
                [6.0 / 16.0, 40.0, 6.0 / 16.0],
                [10.0 / 16.0, 41.0, 10.0 / 16.0]
            )
        );
    }

    #[test]
    fn isolated_pane_mesh_uses_the_centre_post() {
        let mesh = single_block_mesh(BlockState::new(101, 0));
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(
            bounds(&mesh.vertices),
            (
                [7.0 / 16.0, 40.0, 7.0 / 16.0],
                [9.0 / 16.0, 41.0, 9.0 / 16.0]
            )
        );
    }

    #[test]
    fn pane_mesh_connects_to_panes_and_solid_blocks() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(8, 40, 8, BlockState::new(101, 0));
        column.set_block(9, 40, 8, BlockState::STONE);
        column.set_block(8, 40, 7, BlockState::new(102, 0));
        let world = world_with(column.clone());
        let job = ColumnMeshJob::from_column(&column, &world);

        let shape = pane_shape_for_block(&job, 8, 40, 8);
        assert_eq!(shape.as_slice().len(), 3);
        assert!(shape
            .as_slice()
            .iter()
            .any(|part| part.min == [9.0, 0.0, 7.0] && part.max == [16.0, 16.0, 9.0]));
        assert!(shape
            .as_slice()
            .iter()
            .any(|part| part.min == [7.0, 0.0, 0.0] && part.max == [9.0, 16.0, 7.0]));
    }

    #[test]
    fn pane_mesh_connects_across_a_chunk_border() {
        let mut west = ChunkColumn::empty(0, 0);
        west.set_block(15, 40, 8, BlockState::new(101, 0));
        let mut east = ChunkColumn::empty(1, 0);
        east.set_block(0, 40, 8, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(west);
        world.insert(east);
        let job = ColumnMeshJob::from_world((0, 0), &world);

        let shape = pane_shape_for_block(&job, 15, 40, 8);
        assert!(shape
            .as_slice()
            .iter()
            .any(|part| part.min == [9.0, 0.0, 7.0] && part.max == [16.0, 16.0, 9.0]));
    }

    #[test]
    fn door_mesh_emits_a_thin_panel_box() {
        for meta in [0, 8] {
            let mesh = single_block_mesh(BlockState::new(64, meta));
            assert_eq!(mesh.vertices.len(), 24);
            let (min, max) = bounds(&mesh.vertices);
            assert_eq!(max[2] - min[2], 3.0 / 16.0);
            assert_eq!([min[1], min[2]], [40.0, 0.0]);
            assert_eq!([max[1], max[0]], [41.0, 1.0]);
        }
    }

    #[test]
    fn door_mesh_uses_lower_facing_open_and_upper_hinge_metadata() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(4, 40, 4, BlockState::new(64, 4));
        column.set_block(4, 41, 4, BlockState::new(64, 9));
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());

        assert_eq!(mesh.vertices.len(), 48);
        assert_eq!(
            bounds(&mesh.vertices),
            ([4.0 + 13.0 / 16.0, 40.0, 4.0], [5.0, 42.0, 5.0])
        );
    }

    #[test]
    fn torch_mesh_uses_its_small_standing_box() {
        let mesh = single_block_mesh(BlockState::new(50, 0));
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(
            bounds(&mesh.vertices),
            (
                [7.0 / 16.0, 40.0, 7.0 / 16.0],
                [9.0 / 16.0, 40.625, 9.0 / 16.0]
            )
        );
    }

    /// One dropped stack is a single small spinning cube.
    #[test]
    fn a_dropped_item_is_a_small_cube() {
        let items = [ItemDraw {
            position: [10.0, 64.0, -3.0],
            item: 1,
            meta: 0,
        }];
        let mesh = build_item_mesh(&items, &atlas(), 0.0);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);

        // Centred on the block centre, and no larger than the cube's own
        // diagonal once it is rotated.
        let xs: Vec<f32> = mesh.vertices.iter().map(|v| v.position[0]).collect();
        let ys: Vec<f32> = mesh.vertices.iter().map(|v| v.position[1]).collect();
        let zs: Vec<f32> = mesh.vertices.iter().map(|v| v.position[2]).collect();
        let (min_x, max_x) = xs
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!(((min_x + max_x) / 2.0 - 10.5).abs() < 0.001);
        let span = max_x - min_x;
        assert!((0.249..=0.36).contains(&span), "span {span}");
        // The item floats just above the block it sits on rather than sinking
        // into it.
        assert!(ys.iter().cloned().fold(f32::MIN, f32::max) <= 64.4);
        assert!(zs.iter().cloned().fold(f32::MIN, f32::max) >= -3.0);
    }

    #[test]
    fn a_non_block_item_uses_its_item_texture_layer() {
        let items = [ItemDraw {
            position: [0.0, 64.0, 0.0],
            item: 280,
            meta: 0,
        }];
        let textures = atlas();
        let mesh = build_item_mesh(&items, &textures, 0.0);
        assert_eq!(mesh.vertices.len(), 24);
        let stick_layer = textures.layer("item:stick").unwrap();
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.layer == stick_layer));
    }

    #[test]
    fn hidden_faces_are_culled() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, BlockState::STONE);
        column.set_block(0, 41, 0, BlockState::STONE); // covers the top face
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());
        // 6 faces for the lower block (top hidden) + 6 for the upper one,
        // minus the shared pair: 10 faces.
        assert_eq!(mesh.vertices.len(), 10 * 4);
    }

    #[test]
    fn a_partial_shape_does_not_cull_a_whole_neighbour_face() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, BlockState::new(44, 0));
        column.set_block(1, 40, 0, BlockState::STONE);
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());

        // Neither side can discard its full face: the slab only covers half of
        // the adjacent stone face, so conservative overdraw is safer than a hole.
        assert_eq!(mesh.vertices.len(), 12 * 4);
    }

    #[test]
    fn column_mesh_job_reads_the_stronger_packed_block_or_sky_light() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(3, 40, 5, BlockState::STONE);
        column.set_block(4, 40, 5, BlockState::STONE);
        let section = column.section_mut(2).unwrap();
        let mut block_light = section.block_light().to_vec();
        let mut sky_light = vec![0; nephele_world::chunk::SECTION_LIGHT_BYTES];
        set_light_nibble(&mut block_light, 3, 8, 5, 6);
        set_light_nibble(&mut sky_light, 3, 8, 5, 12);
        set_light_nibble(&mut block_light, 4, 8, 5, 13);
        set_light_nibble(&mut sky_light, 4, 8, 5, 4);
        section.set_block_light(&block_light);
        section.set_sky_light(&sky_light);

        let world = world_with(column.clone());
        let job = ColumnMeshJob::from_column(&column, &world);
        assert_eq!(job.light(3, 40, 5), 12);
        assert_eq!(job.light(4, 40, 5), 13);
    }

    #[test]
    fn light_border_snapshot_includes_side_and_diagonal_neighbours() {
        let mut current = ChunkColumn::empty(0, 0);
        current.set_block(15, 40, 8, BlockState::STONE);
        let mut east = ChunkColumn::empty(1, 0);
        east.set_block(0, 40, 8, BlockState::STONE);
        set_block_light(&mut east, 0, 40, 8, 11);
        let mut northwest = ChunkColumn::empty(-1, -1);
        northwest.set_block(15, 40, 15, BlockState::STONE);
        set_block_light(&mut northwest, 15, 40, 15, 7);

        let mut world = World::new(-1);
        world.insert(current);
        world.insert(east);
        world.insert(northwest);
        let job = ColumnMeshJob::from_world((0, 0), &world);

        assert_eq!(job.light(16, 40, 8), 11);
        assert_eq!(job.light(-1, 40, -1), 7);
    }

    #[test]
    fn missing_local_light_arrays_get_direct_sky_but_caves_stay_dark() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(2, 64, 3, BlockState::STONE);
        let world = world_with(column.clone());
        let job = ColumnMeshJob::from_column(&column, &world);

        assert_eq!(job.light(2, 65, 3), 15);
        assert_eq!(job.light(2, 64, 3), 0);
        assert_eq!(job.light(2, 63, 3), 0);
    }

    #[test]
    fn vertex_lighting_averages_four_samples_then_applies_ambient_occlusion() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(4, 40, 4, BlockState::STONE);
        column.set_block(5, 40, 4, BlockState::STONE);
        column.set_block(4, 40, 5, BlockState::STONE);
        let world = world_with(column.clone());
        let job = ColumnMeshJob::from_column(&column, &world);

        let light = vertex_light(&job, [4, 40, 4], &FACES[0], 2);
        assert!((light - 0.55).abs() < 1e-6, "measured corner light {light}");

        let mesh = mesh_column(&job, &atlas());
        assert!((mesh.vertices[2].light - light).abs() < 1e-6);
    }

    #[test]
    fn adjacent_loaded_chunks_cull_across_the_border() {
        let mut east = ChunkColumn::empty(0, 0);
        east.set_block(15, 40, 0, BlockState::STONE);
        let mut west = ChunkColumn::empty(1, 0);
        west.set_block(0, 40, 0, BlockState::STONE);

        let mut world = World::new(0);
        world.insert(east.clone());
        world.insert(west.clone());

        let a = build_column_mesh(&east, &world, &atlas());
        let b = build_column_mesh(&west, &world, &atlas());
        // The touching faces are gone: 5 faces each instead of 6.
        assert_eq!(a.vertices.len(), 20);
        assert_eq!(b.vertices.len(), 20);
    }

    #[test]
    fn border_faces_are_culled_on_all_four_sides() {
        // (neighbour chunk, block inside it, block in our column) — each pair
        // touches across exactly one column border.
        let cases = [
            ((-1, 0), (15, 0), (0, 0)),
            ((1, 0), (0, 0), (15, 0)),
            ((0, -1), (0, 15), (0, 0)),
            ((0, 1), (0, 0), (0, 15)),
        ];
        for (neighbour_coord, (nx, nz), (bx, bz)) in cases {
            let mut column = ChunkColumn::empty(0, 0);
            column.set_block(bx, 40, bz, BlockState::STONE);

            let mut world = World::new(0);
            world.insert(column.clone());
            // On its own the block shows every face.
            assert_eq!(
                build_column_mesh(&column, &world, &atlas()).vertices.len(),
                24,
                "base case {neighbour_coord:?}"
            );

            let mut neighbour = ChunkColumn::empty(neighbour_coord.0, neighbour_coord.1);
            neighbour.set_block(nx, 40, nz, BlockState::STONE);
            world.insert(neighbour);

            // Exactly one face (the shared one) disappears.
            assert_eq!(
                build_column_mesh(&column, &world, &atlas()).vertices.len(),
                5 * 4,
                "neighbour {neighbour_coord:?} should hide one face"
            );
        }
    }

    #[test]
    fn instantly_visible_in_empty_world() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(3, 50, 7, BlockState::STONE);
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());
        // Block occupies [3,4]x[50,51]x[7,8].
        let min = mesh.vertices.iter().fold([f32::MAX; 3], |acc, v| {
            [
                acc[0].min(v.position[0]),
                acc[1].min(v.position[1]),
                acc[2].min(v.position[2]),
            ]
        });
        let max = mesh.vertices.iter().fold([f32::MIN; 3], |acc, v| {
            [
                acc[0].max(v.position[0]),
                acc[1].max(v.position[1]),
                acc[2].max(v.position[2]),
            ]
        });
        assert_eq!(min, [3.0, 50.0, 7.0]);
        assert_eq!(max, [4.0, 51.0, 8.0]);
    }

    #[test]
    fn world_mesh_merges_columns() {
        let mut a = ChunkColumn::empty(0, 0);
        a.set_block(0, 40, 0, BlockState::STONE);
        let mut b = ChunkColumn::empty(1, 0);
        b.set_block(0, 40, 0, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(a);
        world.insert(b);
        let mesh = build_world_mesh(&world, &atlas());
        assert_eq!(mesh.vertices.len(), 48);
        assert_eq!(mesh.indices.len(), 72);
    }

    #[test]
    fn plants_are_crossed_quads_not_cubes() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(3, 64, 3, BlockState::new(2, 0));
        // A flower on top of the grass.
        column.set_block(3, 65, 3, BlockState::new(38, 0));
        let world = World::new(0);
        let job = ColumnMeshJob::from_column(&column, &world);
        let mesh = build_column_mesh(
            &column,
            &world,
            &nephele_assets::TextureArray::from_tiles(Vec::new()),
        );
        // The cube contributes 6 faces, the plant 4 quads (2 shapes x 2 sides);
        // a cube plant would have contributed 6 instead.
        let _ = job;
        let mut quads = 0;
        for window in mesh.indices.chunks(6) {
            let positions: Vec<[f32; 3]> = window
                .iter()
                .map(|index| mesh.vertices[*index as usize].position)
                .collect();
            if positions.iter().all(|p| p[1] >= 65.0) {
                quads += 1;
            }
        }
        assert_eq!(quads, 4, "expected two crossed quads drawn both ways");
    }
    #[test]
    fn mesh_job_snapshot_matches_live_world() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, BlockState::STONE);
        column.set_block(1, 40, 0, BlockState::DIRT);
        let world = world_with(column.clone());
        let job = ColumnMeshJob::from_column(&column, &world);
        assert_eq!(
            mesh_column(&job, &atlas()).vertices.len(),
            build_column_mesh(&column, &world, &atlas()).vertices.len()
        );
        // Out-of-range reads are air, never a panic.
        assert_eq!(job.block(-1, 40, 0), 0);
        assert_eq!(job.block(16, 40, 0), 0);
        assert_eq!(job.block(0, -1, 0), 0);
        assert_eq!(job.block(0, 256, 0), 0);
        assert_eq!(job.light(0, -1, 0), 0);
        assert_eq!(job.light(0, 256, 0), 15);
    }
}
