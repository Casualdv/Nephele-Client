//! Humanoid entity geometry.
//!
//! A vanilla player model is six boxes unwrapped from a 64×64 skin. The layout
//! below is the standard one every skin is authored against, so any downloaded
//! skin lands on the right faces without per-skin configuration.
//!
//! Everything is built in world space each frame — a handful of entities is
//! only a few thousand vertices, and interpolation changes the positions every
//! frame anyway.

use glam::Vec3;
use nephele_assets::SkinAtlas;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EntityVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    /// Face brightness, matching the block renderer's fake directional light.
    pub shade: f32,
    /// Multiplied into the sampled texel; white for players, a tint for mobs.
    pub color: [f32; 4],
}

impl EntityVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Float32,
        3 => Float32x4
    ];

    pub const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EntityVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct EntityMesh {
    pub vertices: Vec<EntityVertex>,
    pub indices: Vec<u32>,
}

/// Which box model to build for a non-player entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobModel {
    Player,
    Biped,
    Enderman,
    Creeper,
    Quadruped,
    Slime,
    Spider,
    Chicken,
    Squid,
    Bat,
}

/// Map protocol-47 mob ids to the models this client can draw.
pub fn mob_model(kind: u8) -> MobModel {
    match kind {
        50 => MobModel::Creeper,
        51 | 54 | 57 => MobModel::Biped,
        52 | 59 => MobModel::Spider,
        55 | 62 => MobModel::Slime,
        58 => MobModel::Enderman,
        65 => MobModel::Bat,
        90..=92 | 95 | 96 => MobModel::Quadruped,
        93 => MobModel::Chicken,
        94 => MobModel::Squid,
        _ => MobModel::Biped,
    }
}

impl EntityMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// One entity to draw this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityDraw {
    /// Feet position in world space.
    pub feet: [f32; 3],
    /// Body yaw in Minecraft degrees (0 = facing +Z).
    pub yaw: f32,
    /// Absolute head yaw in Minecraft degrees.
    pub head_yaw: f32,
    /// Head pitch in Minecraft degrees (positive looks down).
    pub pitch: f32,
    /// Skin atlas slot for player-shaped entities.
    pub slot: u32,
    /// Tint multiplied over the texture; white draws the skin unchanged.
    pub tint: [f32; 4],
    /// Uniform scale; 1.0 is a normal adult.
    pub scale: f32,
    /// Box model to build. Players use the original skin layout.
    pub model: MobModel,
    /// Walk cycle phase in radians. Limbs swing around it.
    pub walk: f32,
    /// Vanilla's crouch flag: lowers the head/body for humanoids.
    pub crouch: bool,
    /// Invisible entities draw at reduced alpha so their nametag/equipment
    /// still has a visible owner.
    pub invisible: bool,
    /// Alex-shaped 3px arms instead of Steve's 4px arms.
    pub slim: bool,
    /// Arm swing phase, 0 = resting, 1 = fully forward. Driven by `S0B`.
    pub arm_swing: f32,
    /// Boots, leggings, chestplate, helmet tints. `None` means that slot is
    /// empty; a tint draws a solid armour box over the matching body part.
    pub armor: [Option<[f32; 4]>; 4],
    /// Map atlas slot for a map held in the main hand, if any.
    pub held_map_slot: Option<u32>,
    /// Draw the hat/jacket/sleeve/pants layer. Off for geometry probes.
    pub overlay: bool,
}

impl EntityDraw {
    pub fn player(feet: [f32; 3], yaw: f32, head_yaw: f32, pitch: f32, slot: u32) -> Self {
        Self {
            feet,
            yaw,
            head_yaw,
            pitch,
            slot,
            tint: [1.0; 4],
            scale: 1.0,
            model: MobModel::Player,
            walk: 0.0,
            crouch: false,
            invisible: false,
            slim: false,
            arm_swing: 0.0,
            armor: [None; 4],
            held_map_slot: None,
            overlay: true,
        }
    }

    pub fn mob(
        feet: [f32; 3],
        yaw: f32,
        head_yaw: f32,
        pitch: f32,
        slot: u32,
        kind: u8,
        walk: f32,
    ) -> Self {
        Self {
            feet,
            yaw,
            head_yaw,
            pitch,
            slot,
            tint: [1.0; 4],
            scale: 1.0,
            model: mob_model(kind),
            walk,
            crouch: false,
            invisible: false,
            slim: false,
            arm_swing: 0.0,
            armor: [None; 4],
            held_map_slot: None,
            overlay: true,
        }
    }
}

/// Vanilla scales player models to 0.9375 of the raw 1/16-block model grid, so a
/// humanoid is 1.875 blocks tall rather than 2.
const PIXEL: f32 = 0.9375 / 16.0;
/// Vanilla's crouch also leans the upper body forward around the hips; the
/// 2-pixel drop alone is almost invisible at lobby distance.
const CROUCH_LEAN: f32 = 0.5;
/// Model y of the head box's bottom face, in pixels - the neck pivot.
const NECK_HEIGHT: f32 = 24.0;

/// A box in model pixels: lower corner, size, and its UV origin on the skin.
#[derive(Clone, Copy)]
struct Part {
    origin: [f32; 3],
    size: [f32; 3],
    uv: [f32; 2],
    /// Overlay (hat/jacket/sleeve/pants) origin on the skin.
    overlay_uv: [f32; 2],
    /// Heads turn independently of the body.
    follows_head: bool,
}

/// The humanoid box layout. Classic skins use 4px arms at x=40/32; slim skins
/// use 3px arms at x=44/36 and sit one pixel closer to the body.
const fn humanoid_parts(slim: bool) -> [Part; 6] {
    let arm_width = if slim { 3.0 } else { 4.0 };
    let right_arm_x = if slim { -7.0 } else { -8.0 };
    let right_arm_uv = if slim { [44.0, 16.0] } else { [40.0, 16.0] };
    let left_arm_uv = if slim { [36.0, 48.0] } else { [32.0, 48.0] };
    let right_arm_overlay = if slim { [44.0, 32.0] } else { [40.0, 32.0] };
    let left_arm_overlay = if slim { [40.0, 48.0] } else { [48.0, 48.0] };
    [
        // Head: 8×8×8 at (0,0), sits on top of the body.
        Part {
            origin: [-4.0, 24.0, -4.0],
            size: [8.0, 8.0, 8.0],
            uv: [0.0, 0.0],
            overlay_uv: [32.0, 0.0],
            follows_head: true,
        },
        // Body: 8×12×4 at (16,16).
        Part {
            origin: [-4.0, 12.0, -2.0],
            size: [8.0, 12.0, 4.0],
            uv: [16.0, 16.0],
            overlay_uv: [16.0, 32.0],
            follows_head: false,
        },
        // Right arm.
        Part {
            origin: [right_arm_x, 12.0, -2.0],
            size: [arm_width, 12.0, 4.0],
            uv: right_arm_uv,
            overlay_uv: right_arm_overlay,
            follows_head: false,
        },
        // Left arm (64×64 skins only).
        Part {
            origin: [4.0, 12.0, -2.0],
            size: [arm_width, 12.0, 4.0],
            uv: left_arm_uv,
            overlay_uv: left_arm_overlay,
            follows_head: false,
        },
        // Right leg: 4×12×4 at (0,16).
        Part {
            origin: [-4.0, 0.0, -2.0],
            size: [4.0, 12.0, 4.0],
            uv: [0.0, 16.0],
            overlay_uv: [0.0, 32.0],
            follows_head: false,
        },
        // Left leg: 4×12×4 at (16,48).
        Part {
            origin: [0.0, 0.0, -2.0],
            size: [4.0, 12.0, 4.0],
            uv: [16.0, 48.0],
            overlay_uv: [0.0, 48.0],
            follows_head: false,
        },
    ]
}

/// Classic layout, kept for the geometry tests.
#[cfg(test)]
const PARTS: [Part; 6] = humanoid_parts(false);

/// A face of a box: which side, the winding order (matching the block mesher so
/// backface culling agrees), and where its texture rectangle starts inside the
/// skin's unwrap.
struct FaceSpec {
    /// Corner positions in normalised box space (0..1 on each axis).
    corners: [[f32; 3]; 4],
    shade: f32,
    /// Rectangle offset inside the 2*(w+d) × (d+h) unwrap, in skin pixels.
    rect: [f32; 2],
    /// Rectangle size, which is one of (w,d), (d,h), (w,h).
    size_kind: RectKind,
}

#[derive(Clone, Copy)]
enum RectKind {
    /// Width × depth — the top and bottom faces.
    TopBottom,
    /// Depth × height — the four side faces.
    SideWh,
    /// Width × height — front and back.
    FrontBack,
}

/// Face order is +Y, -Y, -Z, +Z, +X, -X, with the same corner winding as the
/// block mesher (which the pipeline's cull mode is already tuned to).
const FACES: [FaceSpec; 6] = [
    // +Y (up): u along +X, v along +Z.
    FaceSpec {
        corners: [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        shade: 1.0,
        rect: [0.0, 0.0],
        size_kind: RectKind::TopBottom,
    },
    // -Y (down).
    FaceSpec {
        corners: [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        shade: 0.5,
        rect: [1.0, 0.0],
        size_kind: RectKind::TopBottom,
    },
    // -Z (back): u runs along -X, v along -Y.
    FaceSpec {
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        shade: 0.8,
        rect: [2.0, 1.0],
        size_kind: RectKind::FrontBack,
    },
    // +Z (front): u runs along -X, v along -Y.
    FaceSpec {
        corners: [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        shade: 0.8,
        rect: [1.0, 1.0],
        size_kind: RectKind::FrontBack,
    },
    // +X (right side): u runs along +Z.
    FaceSpec {
        corners: [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
        shade: 0.65,
        rect: [0.0, 1.0],
        size_kind: RectKind::SideWh,
    },
    // -X (left side): u runs along -Z.
    FaceSpec {
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
        shade: 0.65,
        rect: [1.0, 1.0],
        size_kind: RectKind::SideWh,
    },
];

/// Per-corner (u, v) inside a face rectangle, in 0..1, chosen so the texture is
/// not mirrored when viewed from outside the box.
const CORNER_UV: [[[f32; 2]; 4]; 6] = [
    // +Y: (x, z)
    [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
    // -Y: (x, 1 - z)
    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    // -Z: (1 - x, 1 - y)
    [[1.0, 1.0], [1.0, 0.0], [0.0, 0.0], [0.0, 1.0]],
    // +Z: (1 - x, 1 - y)
    [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
    // +X: (z, 1 - y)
    [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
    // -X: (1 - z, 1 - y)
    [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
];

/// Build the mesh for every entity in the frame.
pub fn build_entities(draws: &[EntityDraw]) -> EntityMesh {
    let mut mesh = EntityMesh::default();
    for draw in draws {
        match draw.model {
            MobModel::Player => append_humanoid(&mut mesh, draw),
            model => append_mob(&mut mesh, draw, model),
        }
    }
    mesh
}

/// The first-person arm.
///
/// `right`/`up`/`forward` are the camera's world-space basis. The arm is built
/// in camera space and then placed in the world, so it shares the terrain
/// pipeline's depth test without needing a second render pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandDraw {
    /// Eye position in world space.
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    /// Skin atlas slot; slot 0 is the built-in stand-in skin.
    pub skin_slot: u32,
    /// 0 = resting, 1 = fully swung (mining or attacking).
    pub swing: f32,
}

/// Where the arm ended up, so a held item can sit in the palm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandPose {
    pub palm: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
}

/// Pixel size of the right-arm box in the vanilla skin layout.
const HAND_ARM_UV: [f32; 2] = [40.0, 16.0];

/// Append the first-person arm to `mesh` and return where the hand is.
///
/// The arm hangs from the bottom-right of the view and reaches forward-left.
/// `swing` raises it, which is enough to read as a mining/attack animation.
pub fn append_hand(mesh: &mut EntityMesh, hand: &HandDraw) -> HandPose {
    let eye = Vec3::from(hand.eye);
    let right = Vec3::from(hand.right);
    let up = Vec3::from(hand.up);
    let forward = Vec3::from(hand.forward);

    let reach = (forward + up * (-0.30 + hand.swing * 0.55) + right * -0.18).normalize_or_zero();
    let side = up.cross(reach).normalize_or_zero();
    let normal = reach.cross(side).normalize_or_zero();

    let shoulder = eye + right * 0.34 + up * -0.40 + forward * 0.30;
    let length = 12.0 * PIXEL;
    let width = 4.0 * PIXEL;
    let centre = shoulder + reach * (length * 0.5);

    let part = Part {
        origin: [0.0; 3],
        size: [4.0, 12.0, 4.0],
        uv: HAND_ARM_UV,
        overlay_uv: HAND_ARM_UV,
        follows_head: false,
    };

    for (face_index, face) in FACES.iter().enumerate() {
        let rect = face_rect(&part, face);
        let base = mesh.vertices.len() as u32;

        for (corner_index, corner) in face.corners.iter().enumerate() {
            // Box-local axes: x = side, y = normal (palm facing), z = reach.
            let local = side * ((corner[0] - 0.5) * width)
                + normal * ((corner[1] - 0.5) * width)
                + reach * ((corner[2] - 0.5) * length);
            let uv_in_rect = CORNER_UV[face_index][corner_index];
            let u = rect[0] + uv_in_rect[0] * rect[2];
            let v = rect[1] + uv_in_rect[1] * rect[3];

            mesh.vertices.push(EntityVertex {
                position: (centre + local).to_array(),
                uv: SkinAtlas::uv(hand.skin_slot, u, v),
                shade: face.shade,
                color: [1.0; 4],
            });
        }

        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    HandPose {
        palm: (shoulder + reach * (length * 1.05)).to_array(),
        right: side.to_array(),
        up: normal.to_array(),
        forward: reach.to_array(),
    }
}

#[derive(Clone, Copy)]
struct MobPart {
    origin: [f32; 3],
    size: [f32; 3],
    uv: [f32; 2],
    pivot: [f32; 3],
    angle: [f32; 3],
    follows_head: bool,
}

impl MobPart {
    fn new(origin: [f32; 3], size: [f32; 3], uv: [f32; 2]) -> Self {
        Self {
            origin,
            size,
            uv,
            pivot: [0.0; 3],
            angle: [0.0; 3],
            follows_head: false,
        }
    }

    fn pivot(mut self, pivot: [f32; 3]) -> Self {
        self.pivot = pivot;
        self
    }

    fn angle(mut self, angle: [f32; 3]) -> Self {
        self.angle = angle;
        self
    }

    fn head(mut self) -> Self {
        self.follows_head = true;
        self
    }
}

fn append_mob(mesh: &mut EntityMesh, draw: &EntityDraw, model: MobModel) {
    let mut parts = Vec::new();
    match model {
        MobModel::Biped => {
            let swing = draw.walk.sin() * 0.7;
            let arm = -swing * 0.75;
            let leg = swing;
            let crouch = if draw.crouch { 2.0 } else { 0.0 };
            let drop = [0.0, -crouch, 0.0];
            parts.push(
                MobPart::new([-4.0, 24.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0])
                    .pivot([0.0, 24.0, 0.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-4.0 + drop[0], 12.0 + drop[1], -2.0],
                [8.0, 12.0, 4.0],
                [16.0, 16.0],
            ));
            parts.push(
                MobPart::new([-8.0, 12.0 + drop[1], -2.0], [4.0, 12.0, 4.0], [40.0, 16.0])
                    .pivot([-4.0, 22.0 + drop[1], 0.0])
                    .angle([arm, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([4.0, 12.0 + drop[1], -2.0], [4.0, 12.0, 4.0], [40.0, 16.0])
                    .pivot([4.0, 22.0 + drop[1], 0.0])
                    .angle([-arm, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([-4.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])
                    .pivot([-2.0, 12.0, 0.0])
                    .angle([-leg, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([0.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])
                    .pivot([2.0, 12.0, 0.0])
                    .angle([leg, 0.0, 0.0]),
            );
        }
        MobModel::Enderman => {
            let swing = draw.walk.sin() * 0.35;
            parts.push(
                MobPart::new([-4.0, 24.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0])
                    .pivot([0.0, 24.0, 0.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-4.0, 12.0, -2.0],
                [8.0, 12.0, 4.0],
                [32.0, 16.0],
            ));
            parts.push(
                MobPart::new([-7.0, -6.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0])
                    .pivot([-6.0, 24.0, 0.0])
                    .angle([swing, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([5.0, -6.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0])
                    .pivot([6.0, 24.0, 0.0])
                    .angle([-swing, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([-3.0, -18.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0])
                    .pivot([-2.0, 12.0, 0.0])
                    .angle([-swing, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([1.0, -18.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0])
                    .pivot([2.0, 12.0, 0.0])
                    .angle([swing, 0.0, 0.0]),
            );
        }
        MobModel::Creeper => {
            let swing = draw.walk.sin() * 0.7;
            parts.push(
                MobPart::new([-4.0, 18.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0])
                    .pivot([0.0, 18.0, 0.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-4.0, 6.0, -2.0],
                [8.0, 12.0, 4.0],
                [16.0, 16.0],
            ));
            for (x, z, phase) in [
                (-2.0, 4.0, 1.0),
                (2.0, 4.0, -1.0),
                (-2.0, -4.0, -1.0),
                (2.0, -4.0, 1.0),
            ] {
                parts.push(
                    MobPart::new([x - 2.0, 0.0, z - 2.0], [4.0, 6.0, 4.0], [0.0, 16.0])
                        .pivot([x, 6.0, z])
                        .angle([swing * phase, 0.0, 0.0]),
                );
            }
        }
        MobModel::Quadruped => {
            let swing = draw.walk.sin() * 0.7;
            parts.push(
                MobPart::new([-4.0, 14.0, 6.0], [8.0, 8.0, 8.0], [0.0, 0.0])
                    .pivot([0.0, 18.0, 6.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-5.0, 6.0, -7.0],
                [10.0, 10.0, 16.0],
                [28.0, 8.0],
            ));
            for (x, z, phase) in [
                (-3.0, 7.0, 1.0),
                (3.0, 7.0, -1.0),
                (-3.0, -5.0, -1.0),
                (3.0, -5.0, 1.0),
            ] {
                parts.push(
                    MobPart::new([x - 2.0, 0.0, z - 2.0], [4.0, 6.0, 4.0], [0.0, 16.0])
                        .pivot([x, 6.0, z])
                        .angle([swing * phase, 0.0, 0.0]),
                );
            }
        }
        MobModel::Slime => {
            let squash = 1.0 + draw.walk.sin() * 0.08;
            parts.push(MobPart::new(
                [-4.0, 0.0, -4.0],
                [8.0, 8.0 * squash, 8.0],
                [0.0, 0.0],
            ));
            parts.push(MobPart::new(
                [-3.25, 4.5, 3.5],
                [2.0, 2.0, 1.0],
                [32.0, 0.0],
            ));
            parts.push(MobPart::new([1.25, 4.5, 3.5], [2.0, 2.0, 1.0], [32.0, 4.0]));
        }
        MobModel::Spider => {
            let swing = draw.walk.sin() * 0.5;
            parts.push(
                MobPart::new([-4.0, 4.0, 3.0], [8.0, 8.0, 8.0], [32.0, 4.0])
                    .pivot([0.0, 8.0, 5.0])
                    .head(),
            );
            parts.push(MobPart::new([-3.0, 5.0, -1.0], [6.0, 6.0, 6.0], [0.0, 0.0]));
            parts.push(MobPart::new(
                [-5.0, 5.0, -9.0],
                [10.0, 8.0, 12.0],
                [0.0, 12.0],
            ));
            for (index, phase) in [1.0, -1.0, 0.8, -0.8, -0.8, 0.8, -1.0, 1.0]
                .into_iter()
                .enumerate()
            {
                let side = if index % 2 == 0 { -1.0 } else { 1.0 };
                let z = 2.0 - (index as f32 * 0.9);
                parts.push(
                    MobPart::new(
                        [if side < 0.0 { -15.0 } else { 5.0 }, 6.0, z - 1.0],
                        [10.0, 2.0, 2.0],
                        [18.0, 0.0],
                    )
                    .pivot([4.0 * side, 6.0, z])
                    .angle([0.0, 0.0, swing * phase * side]),
                );
            }
        }
        MobModel::Chicken => {
            let swing = draw.walk.sin() * 0.8;
            parts.push(
                MobPart::new([-2.0, 13.0, 3.0], [4.0, 6.0, 3.0], [0.0, 0.0])
                    .pivot([0.0, 14.0, 4.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-2.0, 13.0, 6.0],
                [4.0, 2.0, 2.0],
                [14.0, 0.0],
            ));
            parts.push(MobPart::new([-3.0, 5.0, -3.0], [6.0, 8.0, 6.0], [0.0, 9.0]));
            parts.push(
                MobPart::new([-2.0, 0.0, -2.0], [3.0, 5.0, 3.0], [26.0, 0.0])
                    .pivot([-0.5, 5.0, 0.0])
                    .angle([swing, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([1.0, 0.0, -2.0], [3.0, 5.0, 3.0], [26.0, 0.0])
                    .pivot([2.5, 5.0, 0.0])
                    .angle([-swing, 0.0, 0.0]),
            );
            parts.push(
                MobPart::new([-4.0, 5.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])
                    .pivot([-4.0, 8.0, 0.0])
                    .angle([0.0, 0.0, draw.walk.cos() * 0.25]),
            );
            parts.push(
                MobPart::new([3.0, 5.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])
                    .pivot([4.0, 8.0, 0.0])
                    .angle([0.0, 0.0, -draw.walk.cos() * 0.25]),
            );
        }
        MobModel::Squid => {
            parts.push(MobPart::new(
                [-6.0, 8.0, -6.0],
                [12.0, 16.0, 12.0],
                [0.0, 0.0],
            ));
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                parts.push(
                    MobPart::new(
                        [angle.cos() * 5.0 - 1.0, -10.0, angle.sin() * 5.0 - 1.0],
                        [2.0, 18.0, 2.0],
                        [48.0, 0.0],
                    )
                    .pivot([angle.cos() * 5.0, 8.0, angle.sin() * 5.0])
                    .angle([draw.walk.cos() * 0.2, 0.0, 0.0]),
                );
            }
        }
        MobModel::Bat => {
            let flap = draw.walk.cos() * 0.8;
            parts.push(
                MobPart::new([-3.0, 16.0, -3.0], [6.0, 6.0, 6.0], [0.0, 0.0])
                    .pivot([0.0, 19.0, 0.0])
                    .head(),
            );
            parts.push(MobPart::new(
                [-3.0, 4.0, -3.0],
                [6.0, 12.0, 6.0],
                [0.0, 16.0],
            ));
            parts.push(
                MobPart::new([-15.0, 8.0, 0.0], [10.0, 8.0, 1.0], [42.0, 0.0])
                    .pivot([-3.0, 10.0, 0.0])
                    .angle([0.0, 0.0, flap]),
            );
            parts.push(
                MobPart::new([5.0, 8.0, 0.0], [10.0, 8.0, 1.0], [42.0, 0.0])
                    .pivot([3.0, 10.0, 0.0])
                    .angle([0.0, 0.0, -flap]),
            );
        }
        MobModel::Player => unreachable!("player uses append_humanoid"),
    }

    for part in parts {
        append_mob_part(mesh, draw, &part);
    }
}

fn append_mob_part(mesh: &mut EntityMesh, draw: &EntityDraw, part: &MobPart) {
    let body_yaw = draw.yaw.to_radians();
    let head_yaw = draw.head_yaw.to_radians();
    let pitch = draw.pitch.to_radians();
    let scale = draw.scale * PIXEL;
    let alpha = if draw.invisible { 0.35 } else { draw.tint[3] };

    let model_part = Part {
        origin: part.origin,
        size: part.size,
        uv: part.uv,
        overlay_uv: part.uv,
        follows_head: part.follows_head,
    };

    for (face_index, face) in FACES.iter().enumerate() {
        let rect = face_rect(&model_part, face);
        let base = mesh.vertices.len() as u32;

        for (corner_index, corner) in face.corners.iter().enumerate() {
            let mut local = [
                part.origin[0] + corner[0] * part.size[0],
                part.origin[1] + corner[1] * part.size[1],
                part.origin[2] + corner[2] * part.size[2],
            ];

            // Limb rotation around its pivot, then the head/body yaw.
            let [ax, ay, az] = part.angle;
            local = rotate_around(local, part.pivot, ax, ay, az);
            if part.follows_head {
                local = rotate_x(local, part.pivot, pitch);
                local = rotate_y(local, head_yaw);
            } else {
                local = rotate_y(local, body_yaw);
            }

            let uv_in_rect = CORNER_UV[face_index][corner_index];
            let u = rect[0] + uv_in_rect[0] * rect[2];
            let v = rect[1] + uv_in_rect[1] * rect[3];

            mesh.vertices.push(EntityVertex {
                position: [
                    draw.feet[0] + local[0] * scale,
                    draw.feet[1] + local[1] * scale,
                    draw.feet[2] + local[2] * scale,
                ],
                uv: SkinAtlas::uv(draw.slot, u, v),
                shade: face.shade,
                color: [draw.tint[0], draw.tint[1], draw.tint[2], alpha],
            });
        }

        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn rotate_around(mut point: [f32; 3], pivot: [f32; 3], x: f32, y: f32, z: f32) -> [f32; 3] {
    if x != 0.0 {
        point = rotate_x(point, pivot, x);
    }
    if y != 0.0 {
        point = rotate_y_point(point, pivot, y);
    }
    if z != 0.0 {
        point = rotate_z(point, pivot, z);
    }
    point
}

fn rotate_x(mut point: [f32; 3], pivot: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    let y = point[1] - pivot[1];
    let z = point[2] - pivot[2];
    point[1] = pivot[1] + y * cos - z * sin;
    point[2] = pivot[2] + y * sin + z * cos;
    point
}

fn rotate_y(point: [f32; 3], angle: f32) -> [f32; 3] {
    rotate_y_point(point, [0.0; 3], angle)
}

fn rotate_y_point(mut point: [f32; 3], pivot: [f32; 3], angle: f32) -> [f32; 3] {
    // Minecraft yaw grows towards -X, a negative rotation about +Y.
    let (sin, cos) = (-angle).sin_cos();
    let x = point[0] - pivot[0];
    let z = point[2] - pivot[2];
    point[0] = pivot[0] + x * cos + z * sin;
    point[2] = pivot[2] - x * sin + z * cos;
    point
}

fn rotate_z(mut point: [f32; 3], pivot: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    let x = point[0] - pivot[0];
    let y = point[1] - pivot[1];
    point[0] = pivot[0] + x * cos - y * sin;
    point[1] = pivot[1] + x * sin + y * cos;
    point
}

fn append_humanoid(mesh: &mut EntityMesh, draw: &EntityDraw) {
    let parts = humanoid_parts(draw.slim);
    let alpha = if draw.invisible { 0.35 } else { draw.tint[3] };
    for (part_index, part) in parts.iter().enumerate() {
        append_humanoid_part(mesh, draw, part, part_index, part.uv, 0.0, alpha);
        // The hat/jacket/sleeve/pants layer is the same box, 0.5px larger,
        // sampling the overlay half of the skin. Transparent overlay pixels
        // are discarded by the fragment shader.
        if draw.overlay {
            append_humanoid_part(mesh, draw, part, part_index, part.overlay_uv, 0.25, alpha);
        }
    }
    append_armor(mesh, draw, &parts);
    if let Some(slot) = draw.held_map_slot {
        append_held_map(mesh, draw, slot);
    }
}

fn append_humanoid_part(
    mesh: &mut EntityMesh,
    draw: &EntityDraw,
    part: &Part,
    part_index: usize,
    uv_origin: [f32; 2],
    inflate: f32,
    alpha: f32,
) {
    let body_yaw = draw.yaw.to_radians();
    let head_yaw = draw.head_yaw.to_radians();
    let pitch = draw.pitch.to_radians();
    let yaw = if part.follows_head {
        head_yaw
    } else {
        body_yaw
    };
    let [ox, oy, oz] = part.origin;
    let [w, h, d] = part.size;
    let geometry = Part {
        origin: [ox - inflate, oy - inflate, oz - inflate],
        size: [w + inflate * 2.0, h + inflate * 2.0, d + inflate * 2.0],
        uv: uv_origin,
        overlay_uv: uv_origin,
        follows_head: part.follows_head,
    };
    let rect_part = Part {
        uv: uv_origin,
        ..*part
    };
    let [gx, gy, gz] = geometry.origin;
    let [gw, gh, gd] = geometry.size;

    for (face_index, face) in FACES.iter().enumerate() {
        let rect = face_rect(&rect_part, face);
        let base = mesh.vertices.len() as u32;
        for (corner_index, corner) in face.corners.iter().enumerate() {
            let mut local = [
                gx + corner[0] * gw,
                gy + corner[1] * gh,
                gz + corner[2] * gd,
            ];
            if draw.crouch && part_index < 4 {
                local[1] -= 2.0;
                local = rotate_x(local, [0.0, 12.0, 0.0], CROUCH_LEAN);
            }
            if part_index >= 2 {
                local = rotate_x(
                    local,
                    part_pivot(draw, part_index),
                    part_angle(draw, part_index),
                );
            }
            if part.follows_head {
                let dy = local[1] - NECK_HEIGHT;
                let dz = local[2];
                let (sin, cos) = pitch.sin_cos();
                local[1] = NECK_HEIGHT + dy * cos - dz * sin;
                local[2] = dy * sin + dz * cos;
            }
            let (sin, cos) = (-yaw).sin_cos();
            let x = local[0] * cos + local[2] * sin;
            let z = -local[0] * sin + local[2] * cos;

            let uv_in_rect = CORNER_UV[face_index][corner_index];
            let u = rect[0] + uv_in_rect[0] * rect[2];
            let v = rect[1] + uv_in_rect[1] * rect[3];
            mesh.vertices.push(EntityVertex {
                position: [
                    draw.feet[0] + x * PIXEL * draw.scale,
                    draw.feet[1] + local[1] * PIXEL * draw.scale,
                    draw.feet[2] + z * PIXEL * draw.scale,
                ],
                uv: SkinAtlas::uv(draw.slot, u, v),
                shade: face.shade,
                color: [draw.tint[0], draw.tint[1], draw.tint[2], alpha],
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn part_angle(draw: &EntityDraw, part_index: usize) -> f32 {
    let swing = draw.walk.sin() * 0.7;
    match part_index {
        2 => -swing * 0.75 - draw.arm_swing * 1.2,
        3 => swing * 0.75,
        4 => swing,
        5 => -swing,
        _ => 0.0,
    }
}

fn part_pivot(draw: &EntityDraw, part_index: usize) -> [f32; 3] {
    let arm = if draw.slim { 5.5 } else { 6.0 };
    match part_index {
        2 => [-arm, 22.0, 0.0],
        3 => [arm, 22.0, 0.0],
        4 => [-2.0, 12.0, 0.0],
        5 => [2.0, 12.0, 0.0],
        _ => [0.0; 3],
    }
}

/// Armour is drawn as a slightly inflated solid box tinted by material. The
/// slot order is boots, leggings, chestplate, helmet.
fn append_armor(mesh: &mut EntityMesh, draw: &EntityDraw, parts: &[Part; 6]) {
    if !draw.armor.iter().any(Option::is_some) {
        return;
    }
    if let Some(color) = draw.armor[0] {
        let mut boot = parts[4];
        boot.origin[1] = 0.0;
        boot.size[1] = 4.0;
        append_solid_part(mesh, draw, &boot, 4, 0.35, color);
        let mut boot = parts[5];
        boot.origin[1] = 0.0;
        boot.size[1] = 4.0;
        append_solid_part(mesh, draw, &boot, 5, 0.35, color);
    }
    if let Some(color) = draw.armor[1] {
        append_solid_part(mesh, draw, &parts[4], 4, 0.3, color);
        append_solid_part(mesh, draw, &parts[5], 5, 0.3, color);
    }
    if let Some(color) = draw.armor[2] {
        append_solid_part(mesh, draw, &parts[1], 1, 0.35, color);
    }
    if let Some(color) = draw.armor[3] {
        append_solid_part(mesh, draw, &parts[0], 0, 0.4, color);
    }
}

fn append_solid_part(
    mesh: &mut EntityMesh,
    draw: &EntityDraw,
    part: &Part,
    part_index: usize,
    inflate: f32,
    color: [f32; 4],
) {
    let [ox, oy, oz] = part.origin;
    let [w, h, d] = part.size;
    let [gx, gy, gz] = [ox - inflate, oy - inflate, oz - inflate];
    let [gw, gh, gd] = [w + inflate * 2.0, h + inflate * 2.0, d + inflate * 2.0];
    let uv = SkinAtlas::uv(SkinAtlas::solid_slot(), 0.0, 0.0);
    let alpha = if draw.invisible { 0.35 } else { color[3] };
    for (face_index, face) in FACES.iter().enumerate() {
        // A helmet is an open-faced shell: leave the +Z face out so the
        // player's face is still visible under it.
        if part_index == 0 && face_index == 3 {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        for corner in face.corners.iter() {
            let mut local = [
                gx + corner[0] * gw,
                gy + corner[1] * gh,
                gz + corner[2] * gd,
            ];
            if draw.crouch && part_index < 4 {
                local[1] -= 2.0;
                local = rotate_x(local, [0.0, 12.0, 0.0], CROUCH_LEAN);
            }
            if part_index >= 2 {
                local = rotate_x(
                    local,
                    part_pivot(draw, part_index),
                    part_angle(draw, part_index),
                );
            }
            if part.follows_head {
                let dy = local[1] - NECK_HEIGHT;
                let dz = local[2];
                let (sin, cos) = draw.pitch.to_radians().sin_cos();
                local[1] = NECK_HEIGHT + dy * cos - dz * sin;
                local[2] = dy * sin + dz * cos;
            }
            let yaw = if part.follows_head {
                draw.head_yaw
            } else {
                draw.yaw
            }
            .to_radians();
            let (sin, cos) = (-yaw).sin_cos();
            let x = local[0] * cos + local[2] * sin;
            let z = -local[0] * sin + local[2] * cos;
            mesh.vertices.push(EntityVertex {
                position: [
                    draw.feet[0] + x * PIXEL * draw.scale,
                    draw.feet[1] + local[1] * PIXEL * draw.scale,
                    draw.feet[2] + z * PIXEL * draw.scale,
                ],
                uv,
                shade: face.shade,
                color: [color[0], color[1], color[2], alpha],
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Where the main-hand item sits. The client uses this to attach a held block
/// or tool; the renderer uses it for a held map.
pub fn humanoid_hand(draw: &EntityDraw) -> HandPose {
    let parts = humanoid_parts(draw.slim);
    let arm = &parts[2];
    let pivot = part_pivot(draw, 2);
    let angle = part_angle(draw, 2);
    let local = rotate_x(
        [
            arm.origin[0] + arm.size[0] * 0.5,
            arm.origin[1],
            arm.origin[2] + arm.size[2] * 0.5,
        ],
        pivot,
        angle,
    );
    let (sin, cos) = (-draw.yaw.to_radians()).sin_cos();
    let x = local[0] * cos + local[2] * sin;
    let z = -local[0] * sin + local[2] * cos;
    let palm = [
        draw.feet[0] + x * PIXEL * draw.scale,
        draw.feet[1] + local[1] * PIXEL * draw.scale,
        draw.feet[2] + z * PIXEL * draw.scale,
    ];
    let right = glam::Vec3::new(cos, 0.0, -sin);
    let up = glam::Vec3::new(0.0, 1.0, 0.0);
    let forward = right.cross(up).normalize_or_zero();
    HandPose {
        palm,
        right: right.to_array(),
        up: up.to_array(),
        forward: forward.to_array(),
    }
}

/// A map held in the first-person hand, using the same atlas slot as framed
/// maps. `size` is the edge length in blocks.
pub fn append_hand_map(mesh: &mut EntityMesh, pose: &HandPose, slot: u32, size: f32) {
    let centre = glam::Vec3::from(pose.palm) + glam::Vec3::from(pose.forward) * 0.06;
    let right = glam::Vec3::from(pose.right) * size * 0.5;
    let up = glam::Vec3::from(pose.up) * size * 0.5;
    append_flat_quad(
        mesh,
        centre,
        right,
        up,
        SkinAtlas::uv(slot, 0.0, 0.0),
        SkinAtlas::uv(slot, 128.0, 128.0),
        [1.0; 4],
    );
}

fn append_held_map(mesh: &mut EntityMesh, draw: &EntityDraw, slot: u32) {
    let hand = humanoid_hand(draw);
    let centre = glam::Vec3::from(hand.palm) + glam::Vec3::from(hand.forward) * 0.08;
    let size = 0.32;
    let right = glam::Vec3::from(hand.right) * size;
    let up = glam::Vec3::from(hand.up) * size;
    let uv = SkinAtlas::uv(slot, 0.0, 0.0);
    let uv2 = SkinAtlas::uv(slot, 128.0, 128.0);
    append_flat_quad(
        mesh,
        centre,
        right,
        up,
        uv,
        uv2,
        [1.0, 1.0, 1.0, if draw.invisible { 0.35 } else { 1.0 }],
    );
}

/// One flat, camera-independent quad used by held maps and item frames.
fn append_flat_quad(
    mesh: &mut EntityMesh,
    centre: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    color: [f32; 4],
) {
    let base = mesh.vertices.len() as u32;
    let corners = [
        (-1.0, -1.0, uv_min[0], uv_max[1]),
        (1.0, -1.0, uv_max[0], uv_max[1]),
        (1.0, 1.0, uv_max[0], uv_min[1]),
        (-1.0, 1.0, uv_min[0], uv_min[1]),
    ];
    for (x, y, u, v) in corners {
        let position = centre + right * x + up * y;
        mesh.vertices.push(EntityVertex {
            position: position.to_array(),
            uv: [u, v],
            shade: 1.0,
            color,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// One item frame, drawn as a flat frame quad with the framed item in front.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemFrameDraw {
    /// Centre of the frame, in world space.
    pub position: [f32; 3],
    /// 0 down, 1 up, 2 north, 3 south, 4 west, 5 east.
    pub direction: u8,
    /// 0..7, each step rotates the item 45°.
    pub rotation: u8,
    pub frame_slot: u32,
    /// Map atlas slot for a map item; `None` draws a tinted placeholder.
    pub item_slot: Option<u32>,
    pub item_tint: [f32; 4],
}

pub fn append_item_frame(mesh: &mut EntityMesh, draw: &ItemFrameDraw) {
    // `direction` is the spawn packet's `EnumFacing.getHorizontalIndex()`:
    // 0 = south (+Z), 1 = west (-X), 2 = north (-Z), 3 = east (+X).
    let normal = match draw.direction & 3 {
        0 => glam::Vec3::Z,
        1 => glam::Vec3::NEG_X,
        2 => glam::Vec3::NEG_Z,
        _ => glam::Vec3::X,
    };
    let up = glam::Vec3::Y;
    let right = up.cross(normal).normalize_or_zero();
    let centre = glam::Vec3::from(draw.position) + normal * 0.02;
    let frame_uv_min = SkinAtlas::uv(draw.frame_slot, 0.0, 0.0);
    let frame_uv_max = SkinAtlas::uv(draw.frame_slot, 16.0, 16.0);
    append_flat_quad(
        mesh,
        centre,
        right * 0.375,
        up * 0.375,
        frame_uv_min,
        frame_uv_max,
        [1.0; 4],
    );
    let (sin, cos) = (draw.rotation as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
    let item_right = right * cos + up * sin;
    let item_up = up * cos - right * sin;
    let item_centre = centre + normal * 0.01;
    let (uv_min, uv_max) = match draw.item_slot {
        Some(slot) => (
            SkinAtlas::uv(slot, 0.0, 0.0),
            SkinAtlas::uv(slot, 128.0, 128.0),
        ),
        None => {
            let uv = SkinAtlas::uv(SkinAtlas::solid_slot(), 0.0, 0.0);
            (uv, uv)
        }
    };
    append_flat_quad(
        mesh,
        item_centre,
        item_right * 0.3,
        item_up * 0.3,
        uv_min,
        uv_max,
        draw.item_tint,
    );
}

/// Texture rectangle `(x, y, w, h)` in skin pixels for one face of `part`.
///
/// The unwrap is `2(w+d)` wide and `d+h` tall: top and bottom along the top
/// row, then left/front/right/back around the middle.
fn face_rect(part: &Part, face: &FaceSpec) -> [f32; 4] {
    let [w, h, d] = part.size;
    let [u, v] = part.uv;
    match face.size_kind {
        // Top sits at u+d, bottom after it at u+d+w.
        RectKind::TopBottom => {
            if face.rect[0] > 0.5 {
                [u + d + w, v, w, d]
            } else {
                [u + d, v, w, d]
            }
        }
        // Front sits between the two sides; the back comes after them.
        RectKind::FrontBack => {
            if face.rect[0] > 1.5 {
                [u + d + w + d, v + d, w, h]
            } else {
                [u + d, v + d, w, h]
            }
        }
        // Right (east) at u, left (west) at u+d+w.
        RectKind::SideWh => {
            if face.rect[0] > 0.5 {
                [u + d + w, v + d, d, h]
            } else {
                [u, v + d, d, h]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_assets::SKIN_SIZE;

    #[test]
    fn the_hand_is_one_skin_textured_box() {
        let mut mesh = EntityMesh::default();
        let pose = append_hand(
            &mut mesh,
            &HandDraw {
                eye: [0.0, 0.0, 0.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                forward: [0.0, 0.0, -1.0],
                skin_slot: 0,
                swing: 0.0,
            },
        );
        assert_eq!(mesh.vertices.len(), 24, "one box is six faces of four");
        assert_eq!(mesh.indices.len(), 36);

        let [u0, v0] = SkinAtlas::uv(0, 0.0, 0.0);
        let [u1, v1] = SkinAtlas::uv(0, SKIN_SIZE as f32, SKIN_SIZE as f32);
        for vertex in &mesh.vertices {
            assert!(vertex.uv[0] >= u0 - 1e-6 && vertex.uv[0] <= u1 + 1e-6);
            assert!(vertex.uv[1] >= v0 - 1e-6 && vertex.uv[1] <= v1 + 1e-6);
        }

        // Forward is -Z: the palm sits in front, below the eye and to the right.
        assert!(pose.palm[2] < -0.2, "hand should be in front: {pose:?}");
        assert!(pose.palm[1] < 0.0, "hand should sit low: {pose:?}");
        assert!(pose.palm[0] > 0.0, "hand should sit right: {pose:?}");
    }

    #[test]
    fn swinging_lifts_the_hand() {
        let hand = |swing| {
            let mut mesh = EntityMesh::default();
            append_hand(
                &mut mesh,
                &HandDraw {
                    eye: [0.0; 3],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    forward: [0.0, 0.0, -1.0],
                    skin_slot: 0,
                    swing,
                },
            )
            .palm
        };
        assert!(hand(1.0)[1] - hand(0.0)[1] > 0.1, "swing raises the palm");
    }

    fn bare_player(feet: [f32; 3], yaw: f32, head_yaw: f32, pitch: f32, slot: u32) -> EntityDraw {
        let mut draw = EntityDraw::player(feet, yaw, head_yaw, pitch, slot);
        draw.overlay = false;
        draw
    }

    #[test]
    fn one_humanoid_is_six_boxes() {
        let mesh = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        assert_eq!(mesh.vertices.len(), 6 * 6 * 4);
        assert_eq!(mesh.indices.len(), 6 * 6 * 6);
    }

    #[test]
    fn crouching_lowers_the_humanoid() {
        let standing = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let mut crouched = bare_player([0.0; 3], 0.0, 0.0, 0.0, 0);
        crouched.crouch = true;
        let crouched = build_entities(&[crouched]);
        let max_y = |mesh: &EntityMesh| {
            mesh.vertices
                .iter()
                .map(|vertex| vertex.position[1])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let drop = max_y(&standing) - max_y(&crouched);
        assert!(
            drop > 2.0 * PIXEL,
            "a crouch lowers the head/body by more than the raw drop, got {drop}"
        );
    }

    #[test]
    fn the_second_layer_doubles_the_humanoid_boxes() {
        let mesh = build_entities(&[EntityDraw::player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        assert_eq!(mesh.vertices.len(), 12 * 6 * 4);
        assert_eq!(mesh.indices.len(), 12 * 6 * 6);
    }

    #[test]
    fn a_humanoid_is_slightly_under_two_blocks_tall() {
        let mesh = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let min_y = mesh
            .vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[1]));
        let max_y = mesh
            .vertices
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[1]));
        assert!(min_y.abs() < 1e-4, "feet sit on the origin: {min_y}");
        assert!((max_y - 32.0 * PIXEL).abs() < 1e-4, "height {max_y}");
        assert!(max_y > 1.8 && max_y < 1.9);
    }

    #[test]
    fn feet_follow_the_draw_position() {
        let mesh = build_entities(&[bare_player([100.0, 64.0, -20.0], 0.0, 0.0, 0.0, 0)]);
        let min_x = mesh
            .vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[0]));
        let max_x = mesh
            .vertices
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[0]));
        // Body plus arms span 16 px = just under one block.
        assert!((max_x - min_x - 16.0 * PIXEL).abs() < 1e-4);
        // Arms stick out to x = -8 px; the head reaches z = -4 px.
        assert!(
            (min_x - (100.0 - 8.0 * PIXEL)).abs() < 1e-4,
            "min_x {min_x}"
        );
        assert!((min_y_of(&mesh) - 64.0).abs() < 1e-4);
        assert!(
            (min_z_of(&mesh) - (-20.0 - 4.0 * PIXEL)).abs() < 1e-3,
            "min_z {}",
            min_z_of(&mesh)
        );
    }

    fn min_y_of(mesh: &EntityMesh) -> f32 {
        mesh.vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[1]))
    }

    fn min_z_of(mesh: &EntityMesh) -> f32 {
        mesh.vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[2]))
    }

    #[test]
    fn yaw_zero_faces_positive_z() {
        // The front face is +Z, so with yaw 0 the face is at positive z.
        let mesh = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let max_z = mesh
            .vertices
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[2]));
        // Head front is at z = 4 px.
        assert!((max_z - 4.0 * PIXEL).abs() < 1e-4);
    }

    #[test]
    fn yaw_ninety_degrees_turns_towards_negative_x() {
        // Minecraft yaw 90° faces -X, so the head front moves to -X.
        let mesh = build_entities(&[bare_player([0.0; 3], 90.0, 90.0, 0.0, 0)]);
        let min_x = mesh
            .vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[0]));
        assert!((min_x + 4.0 * PIXEL).abs() < 1e-4, "front at -X: {min_x}");
    }

    #[test]
    fn all_uvs_stay_inside_their_slot() {
        for slot in 0..4u32 {
            let mesh = build_entities(&[EntityDraw::player([0.0; 3], 0.0, 0.0, 0.0, slot)]);
            let [u0, v0] = SkinAtlas::uv(slot, 0.0, 0.0);
            let [u1, v1] = SkinAtlas::uv(slot, SKIN_SIZE as f32, SKIN_SIZE as f32);
            for vertex in &mesh.vertices {
                assert!(vertex.uv[0] >= u0 - 1e-6 && vertex.uv[0] <= u1 + 1e-6);
                assert!(vertex.uv[1] >= v0 - 1e-6 && vertex.uv[1] <= v1 + 1e-6);
            }
        }
    }

    #[test]
    fn head_pitch_tips_the_head_forward() {
        let flat = build_entities(&[EntityDraw::player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let down = build_entities(&[EntityDraw::player([0.0; 3], 0.0, 0.0, 45.0, 0)]);
        // The head occupies the first 24 vertices; pitching down should lower
        // its front corner.
        let front_z_flat = flat.vertices[..24]
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[2]));
        let lowest_pitched = down.vertices[..24]
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MAX, |acc, y| acc.min(y));
        assert!(front_z_flat > 0.0);
        // Pitching 45° drops the head's leading edge below its resting height.
        assert!(lowest_pitched < NECK_HEIGHT * PIXEL);
    }

    #[test]
    fn scale_shrinks_the_whole_model() {
        let mut draw = bare_player([0.0; 3], 0.0, 0.0, 0.0, 0);
        draw.scale = 0.5;
        let mesh = build_entities(&[draw]);
        let max_y = mesh
            .vertices
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[1]));
        assert!((max_y - 32.0 * PIXEL * 0.5).abs() < 1e-4);
    }

    #[test]
    fn slim_arms_are_narrower_than_classic_arms() {
        let classic = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let mut slim_draw = bare_player([0.0; 3], 0.0, 0.0, 0.0, 0);
        slim_draw.slim = true;
        let slim = build_entities(&[slim_draw]);
        let span = |mesh: &EntityMesh| {
            let min = mesh
                .vertices
                .iter()
                .fold(f32::MAX, |acc, v| acc.min(v.position[0]));
            let max = mesh
                .vertices
                .iter()
                .fold(f32::MIN, |acc, v| acc.max(v.position[0]));
            max - min
        };
        assert!((span(&classic) - 16.0 * PIXEL).abs() < 1e-4);
        assert!((span(&slim) - 14.0 * PIXEL).abs() < 1e-4);
    }

    #[test]
    fn armour_adds_solid_boxes() {
        let mut draw = bare_player([0.0; 3], 0.0, 0.0, 0.0, 0);
        draw.armor[3] = Some([1.0, 0.0, 0.0, 1.0]);
        let mesh = build_entities(&[draw]);
        assert!(mesh.vertices.len() > 6 * 6 * 4);
        // Armour uses the solid-white slot, so its tint survives the shader.
        assert!(mesh
            .vertices
            .iter()
            .any(|v| v.uv == SkinAtlas::uv(SkinAtlas::solid_slot(), 0.0, 0.0)));
    }

    #[test]
    fn an_item_frame_is_two_flat_quads() {
        let mut mesh = EntityMesh::default();
        append_item_frame(
            &mut mesh,
            &ItemFrameDraw {
                position: [0.0; 3],
                direction: 2,
                rotation: 1,
                frame_slot: 0,
                item_slot: Some(2),
                item_tint: [1.0; 4],
            },
        );
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
    }

    #[test]
    fn many_entities_keep_their_indices_consistent() {
        let draws: Vec<_> = (0..3)
            .map(|i| bare_player([i as f32, 0.0, 0.0], 0.0, 0.0, 0.0, 0))
            .collect();
        let mesh = build_entities(&draws);
        assert_eq!(mesh.vertices.len(), 3 * 144);
        let max_index = mesh.indices.iter().copied().max().unwrap() as usize;
        assert_eq!(max_index, mesh.vertices.len() - 1);
    }

    #[test]
    fn every_face_lands_on_painted_skin() {
        // Regression: if a UV rect points at an unpainted (alpha 0) part of the
        // skin, the fragment shader discards it and the model silently vanishes.
        let atlas = SkinAtlas::new();
        let mesh = build_entities(&[bare_player([0.0; 3], 0.0, 0.0, 0.0, 0)]);
        let size = nephele_assets::ATLAS_SIZE;
        // Each face is four vertices; sample the face centre so a UV that sits
        // exactly on a region boundary cannot pick the neighbouring (possibly
        // transparent overlay) texel.
        for quad in mesh.vertices.chunks_exact(4) {
            let u = quad.iter().map(|v| v.uv[0]).sum::<f32>() / 4.0;
            let v = quad.iter().map(|v| v.uv[1]).sum::<f32>() / 4.0;
            let x = ((u * size as f32) as u32).min(size - 1);
            let y = ((v * size as f32) as u32).min(size - 1);
            let index = ((y * size + x) * 4) as usize;
            let alpha = atlas.pixels()[index + 3];
            assert_eq!(alpha, 255, "transparent skin texel at uv ({u},{v})");
        }
    }

    #[test]
    fn face_rects_sit_inside_the_unwrap() {
        // For the head (8×8×8 at uv 0,0) the unwrap is 32 wide and 16 tall.
        let head = &PARTS[0];
        for face in &FACES {
            let [x, y, w, h] = face_rect(head, face);
            assert!(x >= 0.0 && x + w <= 32.0, "x {x} w {w}");
            assert!(y >= 0.0 && y + h <= 16.0, "y {y} h {h}");
        }
        // The body (8×12×4 at 16,16) unwrap is 24 wide and 16 tall.
        let body = &PARTS[1];
        for face in &FACES {
            let [x, y, w, h] = face_rect(body, face);
            assert!(x >= 16.0 && x + w <= 40.0, "x {x} w {w}");
            assert!(y >= 16.0 && y + h <= 32.0, "y {y} h {h}");
        }
    }
}
