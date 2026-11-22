//! Block-entity models: chests, signs and banners (entity texture atlas) and
//! beds (block texture atlas).
//!
//! Vanilla draws these from `ModelChest`, `ModelSign`, `ModelBanner` and the
//! `bed_foot`/`bed_head` block models. The geometry here is copied from those,
//! including the model-pixel origins, texture origins, the Y/Z flip entity
//! models use, and Minecraft's clockwise-about-Y model rotation.
//!
//! Chests, signs and banners emit `EntityVertex` quads because their textures
//! are 64ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â64 and live in `textures/entity`; the renderer draws them with the
//! entity pipeline and a bind group for the entity atlas. Beds use block
//! textures (`bed_feet_*`), so they stay on the block pipeline and are appended
//! to the ordinary chunk mesh.
//!
//! Sign and banner *text* is out of scope: no glyph quads, and banners render
//! only the white base texture (colour and pattern layers come from tile-entity
//! NBT, which this pass does not decode).

use nephele_assets::entity_atlas::{self, EntitySprite};
use nephele_assets::TextureArray;
use nephele_world::block_entity::{BlockEntityKind, ChestSize, SignKind};

use crate::entity::{EntityMesh, EntityVertex};
use crate::mesh::{ColumnMeshJob, MeshData, Vertex, NO_ANIMATION};

/// Face brightness, matching the block renderer's directional shading.
const SHADE_UP: f32 = 1.0;
const SHADE_DOWN: f32 = 0.5;
const SHADE_X: f32 = 0.6;
const SHADE_Z: f32 = 0.8;

const PIXEL: f32 = 1.0 / 16.0;

/// Rotate about Y the way a vanilla block model's `y` rotation works: a 90
/// degree turn takes a north-facing (-Z) model to east (+X). Used by the bed.
fn rotate_y(p: [f32; 3], yaw: f32) -> [f32; 3] {
    let (sin, cos) = yaw.sin_cos();
    [p[0] * cos - p[2] * sin, p[1], p[0] * sin + p[2] * cos]
}

/// Rotate about Y the way `GlStateManager.rotate` does for entity models: +90
/// degrees takes +Z to -X. Block models (`rotate_y`) use the opposite
/// convention, so the chest, sign and banner pass the renderer's `rotate`
/// angle straight in.
fn rotate_gl(p: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    [p[0] * cos + p[2] * sin, p[1], -p[0] * sin + p[2] * cos]
}

/// One vanilla `ModelBox`: pixel bounds already including the rotation point,
/// and the texture origin.
struct ModelBox {
    from: [f32; 3],
    size: [f32; 3],
    tex: [f32; 2],
}

/// A `ModelBox` face: the four corner indices, the UV rect and face brightness.
/// The corner order is vanilla's and is outward-facing; every transform here is
/// orientation-preserving, so the winding survives.
struct BoxFace {
    corners: [u8; 4],
    shade: f32,
}

const BOX_FACES: [BoxFace; 6] = [
    // +X
    BoxFace {
        corners: [5, 1, 2, 6],
        shade: SHADE_X,
    },
    // -X
    BoxFace {
        corners: [0, 4, 7, 3],
        shade: SHADE_X,
    },
    // -Y
    BoxFace {
        corners: [5, 4, 0, 1],
        shade: SHADE_DOWN,
    },
    // +Y
    BoxFace {
        corners: [2, 3, 7, 6],
        shade: SHADE_UP,
    },
    // -Z
    BoxFace {
        corners: [1, 0, 3, 2],
        shade: SHADE_Z,
    },
    // +Z
    BoxFace {
        corners: [4, 5, 6, 7],
        shade: SHADE_Z,
    },
];

/// Append one `ModelBox` to an entity mesh.
///
/// `map` converts model pixels to world space. Face rects follow vanilla
/// `ModelBox`: `+X` takes the `dz`-wide strip after the top, `-X` the first
/// strip, and so on.
fn push_model_box<F>(
    mesh: &mut EntityMesh,
    sprite: &EntitySprite,
    model: &ModelBox,
    map: &F,
    light: f32,
) where
    F: Fn([f32; 3]) -> [f32; 3],
{
    let [dx, dy, dz] = model.size;
    let [tx, ty] = model.tex;
    let [x1, y1, z1] = model.from;
    let (x2, y2, z2) = (x1 + dx, y1 + dy, z1 + dz);
    let corners = [
        [x1, y1, z1],
        [x2, y1, z1],
        [x2, y2, z1],
        [x1, y2, z1],
        [x1, y1, z2],
        [x2, y1, z2],
        [x2, y2, z2],
        [x1, y2, z2],
    ];
    // The six rects, in the order of `BOX_FACES`.
    let rects: [[f32; 4]; 6] = [
        [tx + dz + dx, ty + dz, tx + dz + dx + dz, ty + dz + dy],
        [tx, ty + dz, tx + dz, ty + dz + dy],
        [tx + dz, ty, tx + dz + dx, ty + dz],
        [tx + dz + dx, ty + dz, tx + dz + dx + dx, ty],
        [tx + dz, ty + dz, tx + dz + dx, ty + dz + dy],
        [
            tx + dz + dx + dz,
            ty + dz,
            tx + dz + dx + dz + dx,
            ty + dz + dy,
        ],
    ];

    for (face, rect) in BOX_FACES.iter().zip(rects) {
        // `TexturedQuad` assigns the rect corners to the quad vertices in this
        // order, so the texture is not mirrored.
        let uv_pixels = [
            (rect[2], rect[1]),
            (rect[0], rect[1]),
            (rect[0], rect[3]),
            (rect[2], rect[3]),
        ];
        let positions: [[f32; 3]; 4] =
            std::array::from_fn(|index| corners[face.corners[index] as usize]);
        let world: [[f32; 3]; 4] = positions.map(map);
        let uvs: [[f32; 2]; 4] = uv_pixels.map(|(u, v)| sprite.uv(u, v));

        let base = mesh.vertices.len() as u32;
        for index in 0..4 {
            mesh.vertices.push(EntityVertex {
                position: world[index],
                uv: uvs[index],
                shade: face.shade,
                color: [light, light, light, 1.0],
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// A chest's `ModelChest` geometry in model pixels.
fn chest_boxes(width: f32) -> [ModelBox; 3] {
    let knob_x = if width > 16.0 { 15.0 } else { 7.0 };
    [
        // Body: 14(or 30)ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â10ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â14, rotation point (1,6,1).
        ModelBox {
            from: [1.0, 6.0, 1.0],
            size: [width, 10.0, 14.0],
            tex: [0.0, 19.0],
        },
        // Lid: 14(or 30)ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â5ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â14, rotation point (1,7,15).
        ModelBox {
            from: [1.0, 2.0, 1.0],
            size: [width, 5.0, 14.0],
            tex: [0.0, 0.0],
        },
        // Knob: 2ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â4ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â1, centred on a double chest.
        ModelBox {
            from: [knob_x, 5.0, 0.0],
            size: [2.0, 4.0, 1.0],
            tex: [0.0, 0.0],
        },
    ]
}

fn chest_sprite(id: u16, double: bool) -> &'static EntitySprite {
    let key = match (id, double) {
        // 1.8.9 has no `ender_double.png`: ender chests never join.
        (130, _) => "chest_ender",
        (146, true) => "chest_trapped_double",
        (146, false) => "chest_trapped",
        (_, true) => "chest_normal_double",
        (_, false) => "chest_normal",
    };
    entity_atlas::sprite(key).expect("chest sprite is in the atlas layout")
}

/// `BlockChest` facing metadata: 2 north, 3 south, 4 west, 5 east.
fn chest_yaw(meta: u8) -> f32 {
    match meta {
        2 => std::f32::consts::PI,
        4 => std::f32::consts::FRAC_PI_2,
        5 => -std::f32::consts::FRAC_PI_2,
        _ => 0.0,
    }
}

/// Wall-sign and wall-banner facing: the board hangs on the opposite wall.
fn wall_yaw(meta: u8) -> f32 {
    match meta {
        2 => std::f32::consts::PI,
        4 => std::f32::consts::FRAC_PI_2,
        5 => -std::f32::consts::FRAC_PI_2,
        _ => 0.0,
    }
}

/// A sign model in vanilla pixels. The post adds the stick.
fn sign_boxes(post: bool) -> Vec<ModelBox> {
    let mut boxes = vec![ModelBox {
        from: [-12.0, -14.0, -1.0],
        size: [24.0, 12.0, 2.0],
        tex: [0.0, 0.0],
    }];
    if post {
        boxes.push(ModelBox {
            from: [-1.0, -2.0, -1.0],
            size: [2.0, 14.0, 2.0],
            tex: [0.0, 14.0],
        });
    }
    boxes
}

/// A banner's `ModelBanner` geometry. The slate's rotation point (-32) is baked
/// into `from`.
fn banner_boxes(wall: bool) -> Vec<ModelBox> {
    let mut boxes = vec![
        ModelBox {
            from: [-10.0, -32.0, -2.0],
            size: [20.0, 40.0, 1.0],
            tex: [0.0, 0.0],
        },
        ModelBox {
            from: [-10.0, -32.0, -1.0],
            size: [20.0, 2.0, 2.0],
            tex: [0.0, 42.0],
        },
    ];
    if !wall {
        boxes.push(ModelBox {
            from: [-1.0, -30.0, -1.0],
            size: [2.0, 42.0, 2.0],
            tex: [44.0, 0.0],
        });
    }
    boxes
}

/// Append a chest (normal, trapped or ender) at `origin`, the block's minimum
/// corner. `double` spans two blocks from that corner.
pub fn append_chest(
    mesh: &mut EntityMesh,
    id: u16,
    double: bool,
    meta: u8,
    origin: [f32; 3],
    light: f32,
) {
    let sprite = chest_sprite(id, double);
    let yaw = chest_yaw(meta);
    let map = |p: [f32; 3]| {
        let local = [p[0] * PIXEL - 0.5, p[1] * PIXEL - 0.5, p[2] * PIXEL - 0.5];
        let rotated = rotate_gl(local, yaw);
        [
            origin[0] + rotated[0] + 0.5,
            origin[1] + 0.5 - rotated[1],
            origin[2] + 0.5 - rotated[2],
        ]
    };
    for model in chest_boxes(if double { 30.0 } else { 14.0 }) {
        push_model_box(mesh, sprite, &model, &map, light);
    }
}

/// Append a standing or wall sign.
pub fn append_sign(mesh: &mut EntityMesh, kind: SignKind, meta: u8, origin: [f32; 3], light: f32) {
    let sprite = entity_atlas::sprite("sign").expect("sign sprite");
    const SCALE: f32 = 2.0 / 3.0;
    let wall = kind == SignKind::Wall;
    let yaw = if wall {
        wall_yaw(meta)
    } else {
        f32::from(meta) * std::f32::consts::TAU / 16.0
    };
    let offset = if wall {
        [0.0, -0.3125, -0.4375]
    } else {
        [0.0, 0.0, 0.0]
    };
    let map = |p: [f32; 3]| {
        let local = rotate_gl(
            [
                p[0] * PIXEL * SCALE,
                -p[1] * PIXEL * SCALE,
                -p[2] * PIXEL * SCALE,
            ],
            -yaw,
        );
        [
            origin[0] + 0.5 + local[0] + offset[0],
            origin[1] + 0.5 + local[1] + offset[1],
            origin[2] + 0.5 + local[2] + offset[2],
        ]
    };
    for model in sign_boxes(!wall) {
        push_model_box(mesh, sprite, &model, &map, light);
    }
}

/// Append a standing or wall banner.
pub fn append_banner(mesh: &mut EntityMesh, wall: bool, meta: u8, origin: [f32; 3], light: f32) {
    let sprite = entity_atlas::sprite("banner_base").expect("banner sprite");
    const SCALE: f32 = 2.0 / 3.0;
    let yaw = if wall {
        wall_yaw(meta)
    } else {
        f32::from(meta) * std::f32::consts::TAU / 16.0
    };
    let offset = if wall {
        [0.0, -0.3125, -0.4375]
    } else {
        [0.0, 0.0, 0.0]
    };
    let centre_y = if wall { -0.25 * SCALE } else { 0.0 };
    let map = |p: [f32; 3]| {
        let local = rotate_gl(
            [
                p[0] * PIXEL * SCALE,
                -p[1] * PIXEL * SCALE,
                -p[2] * PIXEL * SCALE,
            ],
            -yaw,
        );
        [
            origin[0] + 0.5 + local[0] + offset[0],
            origin[1] + 0.5 + centre_y + local[1] + offset[1],
            origin[2] + 0.5 + local[2] + offset[2],
        ]
    };
    for model in banner_boxes(wall) {
        push_model_box(mesh, sprite, &model, &map, light);
    }
}

/// Map a model UV rect plus a clockwise texture rotation (degrees) onto the
/// four face corners in block-face order.
fn rotated_face_uvs(rect: [f32; 4], rotation: u16) -> [[f32; 2]; 4] {
    let [u1, v1, u2, v2] = rect;
    let unit = [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    unit.map(|(u, v)| {
        let (u, v) = match rotation {
            90 => (v, 1.0 - u),
            180 => (1.0 - u, 1.0 - v),
            270 => (1.0 - v, u),
            _ => (u, v),
        };
        [u1 + u * (u2 - u1), v1 + v * (v2 - v1)]
    })
}

/// A bed half: the 9/16 body plus the 3/16 underside plane, from
/// `models/block/bed_foot.json` / `bed_head.json`.
#[allow(clippy::too_many_arguments)]
pub fn append_bed(
    mesh: &mut MeshData,
    atlas: &TextureArray,
    head: bool,
    meta: u8,
    origin: [f32; 3],
    light: f32,
) {
    let (top, end, side) = if head {
        ("bed_head_top", "bed_head_end", "bed_head_side")
    } else {
        ("bed_feet_top", "bed_feet_end", "bed_feet_side")
    };
    let layer = |name: &str| atlas.slot_or_missing(name);
    let yaw = f32::from(meta & 3) * std::f32::consts::FRAC_PI_2;
    let map = |p: [f32; 3]| {
        let local = rotate_y([p[0] * PIXEL - 0.5, p[1] * PIXEL, p[2] * PIXEL - 0.5], yaw);
        [
            origin[0] + 0.5 + local[0],
            origin[1] + local[1],
            origin[2] + 0.5 + local[2],
        ]
    };

    // Body 16ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â9ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â16. The top face carries `rotation: 90`; the foot's outer end
    // faces north and the head's faces south.
    let top_corners = [
        [0.0, 9.0, 0.0],
        [0.0, 9.0, 16.0],
        [16.0, 9.0, 16.0],
        [16.0, 9.0, 0.0],
    ];
    let end_corners = if head {
        [
            [16.0, 0.0, 16.0],
            [16.0, 9.0, 16.0],
            [0.0, 9.0, 16.0],
            [0.0, 0.0, 16.0],
        ]
    } else {
        [
            [0.0, 0.0, 0.0],
            [0.0, 9.0, 0.0],
            [16.0, 9.0, 0.0],
            [16.0, 0.0, 0.0],
        ]
    };
    let west_corners = [
        [0.0, 0.0, 16.0],
        [0.0, 9.0, 16.0],
        [0.0, 9.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let east_corners = [
        [16.0, 0.0, 0.0],
        [16.0, 9.0, 0.0],
        [16.0, 9.0, 16.0],
        [16.0, 0.0, 16.0],
    ];
    let underside = [
        [0.0, 3.0, 0.0],
        [16.0, 3.0, 0.0],
        [16.0, 3.0, 16.0],
        [0.0, 3.0, 16.0],
    ];

    /// One bed face: corners, UV rect, texture rotation in degrees, face shade and
    /// block texture name.
    type BedFace = ([[f32; 3]; 4], [f32; 4], u16, f32, &'static str);

    let quads: [BedFace; 5] = [
        (top_corners, [0.0, 16.0, 16.0, 0.0], 90, SHADE_UP, top),
        (end_corners, [0.0, 7.0, 16.0, 16.0], 0, SHADE_Z, end),
        (west_corners, [0.0, 7.0, 16.0, 16.0], 0, SHADE_X, side),
        (east_corners, [16.0, 7.0, 0.0, 16.0], 0, SHADE_X, side),
        (
            underside,
            [0.0, 0.0, 16.0, 16.0],
            0,
            SHADE_DOWN,
            "planks_oak",
        ),
    ];
    for (corners, rect, rotation, shade, name) in quads {
        let positions = corners.map(map);
        let uvs = rotated_face_uvs(rect, rotation);
        push_block_quad(mesh, positions, uvs, shade, layer(name), light);
    }
}

fn push_block_quad(
    mesh: &mut MeshData,
    positions: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    shade: f32,
    layer: u32,
    light: f32,
) {
    let base = mesh.vertices.len() as u32;
    for index in 0..4 {
        mesh.vertices.push(Vertex {
            position: positions[index],
            uv: uvs[index],
            shade,
            layer,
            anim: NO_ANIMATION,
            light,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// What the mesher should do with one block entity state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPlan {
    /// Not a block entity at all.
    NotBlockEntity,
    /// A block entity, but this half is drawn by its partner (the second half
    /// of a double chest).
    Skip,
    /// Draw this model.
    Draw(BlockEntityKind),
}

/// The kind to draw, with a chest's single/double size resolved from the
/// neighbouring block states.
pub fn model_for(id: u16, meta: u8, job: &ColumnMeshJob, x: i32, y: i32, z: i32) -> ModelPlan {
    let Some(kind) = BlockEntityKind::of(id, meta) else {
        return ModelPlan::NotBlockEntity;
    };
    if let BlockEntityKind::Chest { .. } = kind {
        // Only normal and trapped chests join; ender chests never do.
        if id != 54 && id != 146 {
            return ModelPlan::Draw(kind);
        }
        let (dx, dz) = if matches!(meta, 2 | 3) {
            (1, 0)
        } else {
            (0, 1)
        };
        let same = |raw: u16| raw >> 4 == id && (raw & 0x0f) as u8 == meta;
        let forward = same(job.block(x + dx, y, z + dz));
        let backward = same(job.block(x - dx, y, z - dz));
        return if forward {
            // The lower-coordinate half draws the whole pair; the other half
            // must not draw a single chest on top of it.
            if backward {
                ModelPlan::Skip
            } else {
                ModelPlan::Draw(BlockEntityKind::Chest {
                    size: ChestSize::Double,
                })
            }
        } else if backward {
            ModelPlan::Skip
        } else {
            ModelPlan::Draw(BlockEntityKind::Chest {
                size: ChestSize::Single,
            })
        };
    }
    ModelPlan::Draw(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityMesh;

    fn bounds(mesh: &EntityMesh) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
        (min, max)
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-4, "{actual} != {expected}");
    }

    #[test]
    fn a_single_chest_is_a_fourteen_pixel_box_on_the_floor() {
        let mut mesh = EntityMesh::default();
        append_chest(&mut mesh, 54, false, 3, [0.0, 0.0, 0.0], 1.0);
        let (min, max) = bounds(&mesh);
        // Body and lid are 14 wide; the knob reaches the south face.
        assert_close(min[0], 1.0 / 16.0);
        assert_close(max[0], 15.0 / 16.0);
        assert_close(min[1], 0.0);
        // The lid tops out at 14/16: 10-pixel body plus a 5-pixel lid with a
        // one-pixel overlap at the seam.
        assert_close(max[1], 14.0 / 16.0);
        assert_close(max[2], 1.0);
        // Three boxes, six quads each, four vertices per quad.
        assert_eq!(mesh.vertices.len(), 3 * 6 * 4);
        assert_eq!(mesh.indices.len(), 3 * 6 * 6);
    }

    #[test]
    fn a_double_chest_spans_two_blocks() {
        let mut mesh = EntityMesh::default();
        append_chest(&mut mesh, 54, true, 3, [0.0, 0.0, 0.0], 1.0);
        let (min, max) = bounds(&mesh);
        assert_close(min[0], 1.0 / 16.0);
        assert_close(max[0], 31.0 / 16.0);
    }

    #[test]
    fn the_chest_knob_turns_with_the_facing() {
        let knob_edge = |meta: u8| {
            let mut mesh = EntityMesh::default();
            append_chest(&mut mesh, 54, false, meta, [0.0, 0.0, 0.0], 1.0);
            let (min, max) = bounds(&mesh);
            (min, max)
        };
        // 3 south: knob toward +Z; 2 north: toward -Z.
        assert_close(knob_edge(3).1[2], 1.0);
        assert_close(knob_edge(2).0[2], 0.0);
        // 4 west: toward -X; 5 east: toward +X.
        assert_close(knob_edge(4).0[0], 0.0);
        assert_close(knob_edge(5).1[0], 1.0);
    }

    #[test]
    fn a_post_sign_has_a_stick_a_wall_sign_does_not() {
        let mut post = EntityMesh::default();
        append_sign(&mut post, SignKind::Post, 0, [0.0, 0.0, 0.0], 1.0);
        let mut wall = EntityMesh::default();
        append_sign(&mut wall, SignKind::Wall, 3, [0.0, 0.0, 0.0], 1.0);
        assert_eq!(post.vertices.len(), 2 * 6 * 4);
        assert_eq!(wall.vertices.len(), 6 * 4);
        // The board is 2/3 scale, 24 pixels wide: one block.
        let (min, max) = bounds(&wall);
        assert_close(max[0] - min[0], 1.0);
        // Board sits from 0.2708 to 0.7708 on the wall.
        assert_close(min[1], 0.2708);
        assert_close(max[1], 0.7708);
    }

    #[test]
    fn a_bed_is_nine_sixteenths_high() {
        let atlas = TextureArray::from_tiles(Vec::new());
        let mut mesh = MeshData::default();
        append_bed(&mut mesh, &atlas, false, 0, [0.0, 0.0, 0.0], 1.0);
        let mut max_y = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        for vertex in &mesh.vertices {
            min_y = min_y.min(vertex.position[1]);
            max_y = max_y.max(vertex.position[1]);
        }
        assert_close(min_y, 0.0);
        assert_close(max_y, 9.0 / 16.0);
        // Five quads: top, one end, two sides, the underside plane.
        assert_eq!(mesh.vertices.len(), 5 * 4);
    }
}

#[cfg(test)]
mod winding_tests {
    use super::*;
    use crate::entity::EntityMesh;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    #[test]
    fn every_box_triangle_faces_away_from_the_box_centre() {
        let sprite = entity_atlas::sprite("chest_normal").expect("sprite");
        let model = ModelBox {
            from: [0.0, 0.0, 0.0],
            size: [14.0, 10.0, 14.0],
            tex: [0.0, 19.0],
        };
        let mut mesh = EntityMesh::default();
        push_model_box(&mut mesh, sprite, &model, &|p| p, 1.0);
        let centre = [7.0, 5.0, 7.0];
        for triangle in mesh.indices.chunks(3) {
            let points: Vec<[f32; 3]> = triangle
                .iter()
                .map(|index| mesh.vertices[*index as usize].position)
                .collect();
            let normal = cross(
                [
                    points[1][0] - points[0][0],
                    points[1][1] - points[0][1],
                    points[1][2] - points[0][2],
                ],
                [
                    points[2][0] - points[0][0],
                    points[2][1] - points[0][1],
                    points[2][2] - points[0][2],
                ],
            );
            let centroid = [
                (points[0][0] + points[1][0] + points[2][0]) / 3.0 - centre[0],
                (points[0][1] + points[1][1] + points[2][1]) / 3.0 - centre[1],
                (points[0][2] + points[1][2] + points[2][2]) / 3.0 - centre[2],
            ];
            let facing =
                normal[0] * centroid[0] + normal[1] * centroid[1] + normal[2] * centroid[2];
            assert!(facing > 0.0, "triangle faces inward: {normal:?}");
        }
    }
}

#[cfg(test)]
mod bed_winding_tests {
    use super::*;
    use crate::mesh::MeshData;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    #[test]
    fn every_bed_quad_faces_outward() {
        let atlas = TextureArray::from_tiles(Vec::new());
        let mut mesh = MeshData::default();
        append_bed(&mut mesh, &atlas, false, 0, [0.0, 0.0, 0.0], 1.0);
        let centre = [0.5, 4.5 / 16.0, 0.5];
        for quad in mesh.indices.chunks(6) {
            let points: Vec<[f32; 3]> = quad[..3]
                .iter()
                .map(|index| mesh.vertices[*index as usize].position)
                .collect();
            let normal = cross(
                [
                    points[1][0] - points[0][0],
                    points[1][1] - points[0][1],
                    points[1][2] - points[0][2],
                ],
                [
                    points[2][0] - points[0][0],
                    points[2][1] - points[0][1],
                    points[2][2] - points[0][2],
                ],
            );
            let centroid = [
                (points[0][0] + points[1][0] + points[2][0]) / 3.0 - centre[0],
                (points[0][1] + points[1][1] + points[2][1]) / 3.0 - centre[1],
                (points[0][2] + points[1][2] + points[2][2]) / 3.0 - centre[2],
            ];
            let facing =
                normal[0] * centroid[0] + normal[1] * centroid[1] + normal[2] * centroid[2];
            assert!(facing > 0.0, "bed quad faces inward: {normal:?}");
        }
    }
}
