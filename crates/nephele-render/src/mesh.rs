use nephele_assets::{
    face_textures, is_transparent, item_texture_name, TextureArray, MISSING_TEXTURE,
};
use nephele_world::shapes::{block_shape, is_full_cube};
use nephele_world::{BlockState, ChunkColumn, ChunkCoord, World};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub shade: f32,
    /// Texture-array layer for this face.
    pub layer: u32,
}

impl Vertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32, 3 => Uint32];

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
}

impl ColumnMeshJob {
    pub fn from_world(key: (i32, i32), world: &World) -> Self {
        let mut sections = vec![None; nephele_world::SECTION_COUNT];
        if let Some(column) = world.get(ChunkCoord::new(key.0, key.1)) {
            for (index, section) in column.iter_sections() {
                if section.non_air_count() > 0 {
                    sections[index] = Some(section.blocks().to_vec());
                }
            }
        }
        Self {
            key,
            sections,
            west: side_plane(world, key.0 - 1, key.1, Axis::X, 15),
            east: side_plane(world, key.0 + 1, key.1, Axis::X, 0),
            north: side_plane(world, key.0, key.1 - 1, Axis::Z, 15),
            south: side_plane(world, key.0, key.1 + 1, Axis::Z, 0),
        }
    }

    pub fn from_column(column: &ChunkColumn, world: &World) -> Self {
        let mut job = Self::from_world((column.x, column.z), world);
        for (index, section) in column.iter_sections() {
            if section.non_air_count() > 0 {
                job.sections[index] = Some(section.blocks().to_vec());
            }
        }
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
        // Exactly one axis stepped out of the column; faces never move two.
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
        // Diagonal neighbours are never queried.
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

/// Mesh a chunk column from an owned snapshot. Pure computation — no world or
/// GPU access — so it is safe to run on any thread.
pub fn mesh_column(job: &ColumnMeshJob, atlas: &TextureArray) -> MeshData {
    let mut mesh = MeshData::default();
    // Block states repeat heavily, so resolve the texture layers once per
    // distinct state instead of three hash lookups per block.
    let mut resolved: std::collections::HashMap<u16, Option<[u32; 3]>> =
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
                        [
                            atlas.layer_or_missing(textures.top),
                            atlas.layer_or_missing(textures.bottom),
                            atlas.layer_or_missing(textures.side),
                        ]
                    });
                    *entry.insert(textures)
                }
            };
            let Some(layers) = layers else {
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
                let bx = (base_x + local_x) as f32;
                let by = y as f32;
                let bz = (base_z + local_z) as f32;
                let (lo, hi) = (0.1f32, 0.9f32);
                let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

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
            let shape = block_shape(id, meta);
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
                    append_quad(&mut mesh, face, layers[face.slot], positions);
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
                        append_quad(&mut mesh, face, layers[face.slot], positions);
                    }
                }
            }
        }
    }

    mesh
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

fn append_quad(mesh: &mut MeshData, face: &FaceDef, layer: u32, positions: [[f32; 3]; 4]) {
    let base = mesh.vertices.len() as u32;
    for (position, uv) in positions.into_iter().zip(face.uvs.iter().copied()) {
        mesh.vertices.push(Vertex {
            position,
            uv,
            shade: face.shade,
            layer,
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
    fn pane_mesh_uses_its_thin_axis_aligned_box() {
        let mesh = single_block_mesh(BlockState::new(101, 0));
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(
            bounds(&mesh.vertices),
            ([7.0 / 16.0, 40.0, 0.0], [9.0 / 16.0, 41.0, 1.0])
        );
    }

    #[test]
    fn door_mesh_emits_a_thin_panel_box() {
        for meta in [0, 8] {
            let mesh = single_block_mesh(BlockState::new(64, meta));
            assert_eq!(mesh.vertices.len(), 24);
            let (min, max) = bounds(&mesh.vertices);
            assert_eq!(max[0] - min[0], 3.0 / 16.0);
            assert_eq!([min[1], min[2]], [40.0, 0.0]);
            assert_eq!([max[1], max[2]], [41.0, 1.0]);
        }
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
    }
}
