use nephele_assets::{face_textures, is_transparent, TextureArray};
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
            for face in &FACES {
                let neighbour = job.block(
                    local_x + face.normal[0],
                    y + face.normal[1],
                    local_z + face.normal[2],
                );
                if !is_transparent(neighbour >> 4) {
                    continue;
                }
                // Two of the same see-through block still hide their shared face.
                if neighbour == raw && is_transparent(id) {
                    continue;
                }

                let layer = layers[face.slot];
                let base = mesh.vertices.len() as u32;
                let x = (base_x + local_x) as f32;
                let yf = y as f32;
                let z = (base_z + local_z) as f32;

                for (corner, uv) in face.corners.iter().zip(face.uvs.iter()) {
                    mesh.vertices.push(Vertex {
                        position: [x + corner[0], yf + corner[1], z + corner[2]],
                        uv: *uv,
                        shade: face.shade,
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
    }

    mesh
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use nephele_assets::TextureArray;
    use nephele_world::ChunkColumn;

    fn atlas() -> TextureArray {
        TextureArray::from_tiles(vec![(
            "stone",
            RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255])),
        )])
    }

    fn world_with(column: ChunkColumn) -> World {
        let mut world = World::new(0);
        world.insert(column);
        world
    }

    #[test]
    fn single_block_emits_six_faces() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 40, 0, BlockState::STONE);
        let world = world_with(column.clone());
        let mesh = build_column_mesh(&column, &world, &atlas());
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
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
