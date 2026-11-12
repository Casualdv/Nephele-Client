//! Block debris, note-block and server-triggered sprite particles.
//!
//! All of them live on renderer paths that already exist. Debris and sprite
//! quads are textured [`MeshData`] uploaded into the break-overlay buffer
//! (same shader, same texture array, no depth writes); notes are camera-facing
//! quads in the [`EntityMesh`] buffer, which already carries per-vertex colour
//! and alpha. Sprite layers come from [`nephele_assets::particle_textures`] -
//! generated tiles appended to the same [`TextureArray`] as the block and item
//! layers, so there is still one particle system and one texture array.
//!
//! Positions here are in blocks and velocities in blocks per second; the
//! simulation runs at display rate rather than 20 Hz because it only feeds the
//! next frame.

use nephele_assets::{
    face_textures, is_transparent, item_texture_name, particle_textures, TextureArray,
    MISSING_TEXTURE,
};
use nephele_world::block::blocks_movement;
use nephele_world::{BlockState, World};

use glam::Vec3;

use crate::entity::{EntityMesh, EntityVertex};
use crate::mesh::{MeshData, Vertex, NO_ANIMATION};

/// One kind of camera-facing particle sprite.
///
/// Block and item cracks sample the block/item texture; the rest use the
/// generated `particle_textures` tiles. This is the same particle path as the
/// block debris above — one `Particles` owner, one texture array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    BlockBreak(BlockState),
    ItemBreak { id: i16, meta: i16 },
    Smoke,
    Flame,
    Crit,
    Portal,
    Splash,
    Heart,
}

impl ParticleKind {
    fn layer(self, textures: &TextureArray) -> u32 {
        match self {
            ParticleKind::BlockBreak(state) => face_textures(state)
                .map(|faces| textures.slot_or_missing(faces.side))
                .unwrap_or_else(|| textures.slot_or_missing("stone")),
            ParticleKind::ItemBreak { id, meta } => item_texture_name(id, meta)
                .map(|name| textures.slot_or_missing(name))
                .unwrap_or_else(|| textures.slot_or_missing(MISSING_TEXTURE)),
            ParticleKind::Smoke => textures.slot_or_missing(particle_textures::SMOKE),
            ParticleKind::Flame => textures.slot_or_missing(particle_textures::FLAME),
            ParticleKind::Crit => textures.slot_or_missing(particle_textures::CRIT),
            ParticleKind::Portal => textures.slot_or_missing(particle_textures::PORTAL),
            ParticleKind::Splash => textures.slot_or_missing(particle_textures::SPLASH),
            ParticleKind::Heart => textures.slot_or_missing(particle_textures::HEART),
        }
    }

    fn base_size(self) -> f32 {
        match self {
            ParticleKind::ItemBreak { .. } => 0.12,
            ParticleKind::BlockBreak(_) => 0.16,
            ParticleKind::Splash => 0.12,
            ParticleKind::Crit => 0.18,
            ParticleKind::Smoke => 0.18,
            ParticleKind::Portal => 0.14,
            ParticleKind::Flame => 0.16,
            ParticleKind::Heart => 0.16,
        }
    }
}

/// One sprite ready for [`build_particle_mesh`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleDraw {
    pub position: [f32; 3],
    /// Edge length in blocks.
    pub size: f32,
    pub kind: ParticleKind,
}

#[derive(Debug, Clone, Copy)]
struct Sprite {
    position: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    life: f32,
    size: f32,
    kind: ParticleKind,
}

/// How long a broken block's shards live, in seconds (vanilla: 20..40 ticks).
const DEBRIS_LIFETIME: f32 = 1.1;
/// Shards spawned for one broken block.
const DEBRIS_COUNT: usize = 14;
/// Downward acceleration on debris, blocks/s^2.
const DEBRIS_GRAVITY: f32 = 24.0;
/// Per-tick horizontal drag, applied as `0.98^(20 dt)`.
const DEBRIS_DRAG: f32 = 0.98;
/// A note particle lives six ticks, like `EntityNoteFX`.
const NOTE_LIFETIME: f32 = 0.3;
/// Initial upward speed of a note, blocks/s (vanilla: 0.2 blocks/tick).
const NOTE_RISE: f32 = 4.0;

#[derive(Debug, Clone, Copy)]
struct Debris {
    position: [f32; 3],
    prev: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    size: f32,
    layer: u32,
    /// Sub-rectangle of the block tile, in 0..1 tile space.
    uv: [f32; 4],
    light: f32,
}

#[derive(Debug, Clone, Copy)]
struct Note {
    position: [f32; 3],
    prev: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    color: [f32; 3],
}

/// Everything transient the client spawns: break debris, note particles and
/// server-triggered sprite bursts.
#[derive(Debug, Clone, Default)]
pub struct Particles {
    debris: Vec<Debris>,
    notes: Vec<Note>,
    sprites: Vec<Sprite>,
    /// Deterministic counter, so a spawn sequence is reproducible in tests.
    seed: u32,
}

impl Particles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.debris.is_empty() && self.notes.is_empty() && self.sprites.is_empty()
    }

    pub fn clear(&mut self) {
        self.debris.clear();
        self.notes.clear();
        self.sprites.clear();
    }

    pub fn debris_count(&self) -> usize {
        self.debris.len()
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    pub fn sprite_count(&self) -> usize {
        self.sprites.len()
    }

    /// Spawn `count` camera-facing sprites of one kind inside `offset`.
    pub fn spawn_kind(
        &mut self,
        kind: ParticleKind,
        position: [f32; 3],
        offset: [f32; 3],
        speed: f32,
        count: i32,
    ) {
        let count = count.clamp(0, 128);
        let speed = speed.max(0.05);
        for _ in 0..count {
            let seed = self.next_seed();
            let random = |salt: u32| rand01(seed ^ salt) * 2.0 - 1.0;
            let particle = [
                position[0] + random(0x9e37_79b9) * offset[0],
                position[1] + random(0x85eb_ca6b) * offset[1],
                position[2] + random(0xc2b2_ae35) * offset[2],
            ];
            let velocity = [
                random(0x27d4_eb2f) * speed,
                rand01(seed ^ 0x1656_67b1) * speed,
                random(0x2545_f491) * speed,
            ];
            let life = 0.35 + rand01(seed ^ 0x7feb_352d) * 0.55;
            let size = kind.base_size() * (0.7 + rand01(seed ^ 0x846c_a68b) * 0.6);
            self.sprites.push(Sprite {
                position: particle,
                velocity,
                age: 0.0,
                life,
                size,
                kind,
            });
        }
    }

    /// Spawn `DEBRIS_COUNT` shards for a block that just broke or was placed.
    ///
    /// Textures are resolved here so the simulation never needs the atlas.
    pub fn spawn_block(
        &mut self,
        textures: &TextureArray,
        state: BlockState,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let Some(faces) = face_textures(state) else {
            return;
        };
        // Vanilla's block particles use the side texture of the block.
        let layer = textures.slot_or_missing(faces.side);
        let center = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
        for _ in 0..DEBRIS_COUNT {
            let seed = self.next_seed();
            let inside = [
                x as f32 + 0.15 + rand01(seed) * 0.7,
                y as f32 + 0.15 + rand01(seed ^ 0x9e37_79b9) * 0.7,
                z as f32 + 0.15 + rand01(seed ^ 0x85eb_ca6b) * 0.7,
            ];
            let mut direction = [
                inside[0] - center[0],
                inside[1] - center[1] + 0.25,
                inside[2] - center[2],
            ];
            let length = (direction[0] * direction[0]
                + direction[1] * direction[1]
                + direction[2] * direction[2])
                .sqrt()
                .max(0.001);
            let speed = 1.2 + rand01(seed ^ 0xc2b2_ae35) * 1.8;
            direction = [
                direction[0] / length * speed,
                direction[1] / length * speed + 1.2,
                direction[2] / length * speed,
            ];
            // A random 4x4 texel window, the same granularity vanilla's
            // `EntityDiggingFX` uses.
            let uv_x = rand01(seed ^ 0x27d4_eb2f) * 12.0 / 16.0;
            let uv_y = rand01(seed ^ 0x1656_67b1) * 12.0 / 16.0;
            self.debris.push(Debris {
                position: inside,
                prev: inside,
                velocity: direction,
                age: 0.0,
                size: 0.09 + rand01(seed ^ 0x7feb_352d) * 0.06,
                layer,
                uv: [uv_x, uv_y, 4.0 / 16.0, 4.0 / 16.0],
                light: 0.85 + rand01(seed ^ 0x2545_f491) * 0.15,
            });
        }
    }

    /// Spawn the note particle a note block emits, coloured by its pitch.
    pub fn spawn_note(&mut self, x: i32, y: i32, z: i32, pitch: u8) {
        // `note` is 0..24; vanilla passes `note / 24` into the hue.
        let hue = (pitch.min(24) as f32) / 24.0;
        let position = [x as f32 + 0.5, y as f32 + 1.2, z as f32 + 0.5];
        self.notes.push(Note {
            position,
            prev: position,
            velocity: [0.0, NOTE_RISE, 0.0],
            age: 0.0,
            color: note_color(hue),
        });
    }

    /// Advance both particle sets. `world` is only used to stop debris falling
    /// through the floor.
    pub fn update(&mut self, dt: f32, world: &World) {
        let dt = dt.clamp(0.0, 0.05);
        if dt <= 0.0 {
            return;
        }
        let drag = DEBRIS_DRAG.powf(dt * 20.0);
        self.debris.retain_mut(|debris| {
            debris.age += dt;
            if debris.age >= DEBRIS_LIFETIME {
                return false;
            }
            debris.prev = debris.position;
            debris.velocity[1] -= DEBRIS_GRAVITY * dt;
            debris.velocity[0] *= drag;
            debris.velocity[2] *= drag;
            let mut next = [
                debris.position[0] + debris.velocity[0] * dt,
                debris.position[1] + debris.velocity[1] * dt,
                debris.position[2] + debris.velocity[2] * dt,
            ];
            if debris.velocity[1] < 0.0 {
                let below = world.get_block(
                    next[0].floor() as i32,
                    (next[1] - 0.05).floor() as i32,
                    next[2].floor() as i32,
                );
                if blocks_movement(below) && !is_transparent(below.id()) {
                    next[1] = next[1].floor() + 0.03;
                    debris.velocity[1] = 0.0;
                    debris.velocity[0] *= 0.5;
                    debris.velocity[2] *= 0.5;
                }
            }
            debris.position = next;
            true
        });

        let note_drag = 0.66f32.powf(dt * 20.0);
        self.notes.retain_mut(|note| {
            note.age += dt;
            if note.age >= NOTE_LIFETIME {
                return false;
            }
            note.prev = note.position;
            note.velocity[1] *= note_drag;
            note.position[0] += note.velocity[0] * dt;
            note.position[1] += note.velocity[1] * dt;
            note.position[2] += note.velocity[2] * dt;
            true
        });

        self.sprites.retain_mut(|sprite| {
            sprite.age += dt;
            if sprite.age >= sprite.life {
                return false;
            }
            sprite.velocity[1] -= 9.8 * dt;
            sprite.position[0] += sprite.velocity[0] * dt;
            sprite.position[1] += sprite.velocity[1] * dt;
            sprite.position[2] += sprite.velocity[2] * dt;
            sprite.position[1] > -64.0
        });
    }

    /// Camera-facing quads for the sprite particles, on the same block texture
    /// array as the debris.
    pub fn build_sprites(
        &self,
        textures: &TextureArray,
        right: [f32; 3],
        up: [f32; 3],
    ) -> MeshData {
        let draws: Vec<ParticleDraw> = self
            .sprites
            .iter()
            .map(|sprite| ParticleDraw {
                position: sprite.position,
                size: sprite.size * (1.0 - sprite.age / sprite.life * 0.35),
                kind: sprite.kind,
            })
            .collect();
        build_particle_mesh(&draws, textures, right, up)
    }

    /// Textured quads for the overlay pass. `alpha` is the fraction through the
    /// current tick so the shards move smoothly between updates.
    pub fn build_debris(&self, right: [f32; 3], up: [f32; 3], alpha: f32) -> MeshData {
        let mut mesh = MeshData::default();
        for debris in &self.debris {
            let centre = [
                debris.prev[0] + (debris.position[0] - debris.prev[0]) * alpha,
                debris.prev[1] + (debris.position[1] - debris.prev[1]) * alpha,
                debris.prev[2] + (debris.position[2] - debris.prev[2]) * alpha,
            ];
            let half = debris.size * 0.5;
            let right = [right[0] * half, right[1] * half, right[2] * half];
            let up = [up[0] * half, up[1] * half, up[2] * half];
            // Counter-clockwise as seen from the camera: the block pipeline
            // culls back faces, so a clockwise billboard would disappear.
            let corners = [
                [
                    centre[0] - right[0] - up[0],
                    centre[1] - right[1] - up[1],
                    centre[2] - right[2] - up[2],
                ],
                [
                    centre[0] + right[0] - up[0],
                    centre[1] + right[1] - up[1],
                    centre[2] + right[2] - up[2],
                ],
                [
                    centre[0] + right[0] + up[0],
                    centre[1] + right[1] + up[1],
                    centre[2] + right[2] + up[2],
                ],
                [
                    centre[0] - right[0] + up[0],
                    centre[1] - right[1] + up[1],
                    centre[2] - right[2] + up[2],
                ],
            ];
            let [u0, v0, u1, v1] = debris.uv;
            let uvs = [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];
            let base = mesh.vertices.len() as u32;
            for (corner, uv) in corners.into_iter().zip(uvs) {
                mesh.vertices.push(Vertex {
                    position: corner,
                    uv,
                    shade: 1.0,
                    layer: debris.layer,
                    anim: NO_ANIMATION,
                    light: debris.light,
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }

    /// Add the note particles to the entity mesh, which carries per-vertex
    /// colour. `(-1, -1)` UV tells the entity shader to draw an untextured,
    /// unfogged quad.
    pub fn append_notes(&self, mesh: &mut EntityMesh, right: [f32; 3], up: [f32; 3], alpha: f32) {
        for note in &self.notes {
            let centre = [
                note.prev[0] + (note.position[0] - note.prev[0]) * alpha,
                note.prev[1] + (note.position[1] - note.prev[1]) * alpha,
                note.prev[2] + (note.position[2] - note.prev[2]) * alpha,
            ];
            // Vanilla's note starts tiny and grows over its six ticks.
            let grown = (note.age / NOTE_LIFETIME).clamp(0.0, 1.0);
            let scale = 0.35 + grown * 0.65;
            let color = [
                note.color[0],
                note.color[1],
                note.color[2],
                1.0 - grown * 0.35,
            ];
            // A note glyph from two billboard quads: the head contains the
            // stem.
            let head_half = [0.07 * scale, 0.05 * scale];
            let stem_half = [0.014 * scale, 0.11 * scale];
            let stem_offset = [0.056 * scale, 0.12 * scale];
            append_quad(mesh, centre, [head_half[0], head_half[1]], right, up, color);
            append_quad(
                mesh,
                [
                    centre[0] + right[0] * stem_offset[0] + up[0] * stem_offset[1],
                    centre[1] + right[1] * stem_offset[0] + up[1] * stem_offset[1],
                    centre[2] + right[2] * stem_offset[0] + up[2] * stem_offset[1],
                ],
                [stem_half[0], stem_half[1]],
                right,
                up,
                color,
            );
        }
    }

    fn next_seed(&mut self) -> u32 {
        self.seed = self.seed.wrapping_add(1).wrapping_mul(1_664_525);
        self.seed ^ 0x9e37_79b9
    }
}

/// Build camera-facing quads for every sprite particle.
pub fn build_particle_mesh(
    particles: &[ParticleDraw],
    textures: &TextureArray,
    right: [f32; 3],
    up: [f32; 3],
) -> MeshData {
    let mut mesh = MeshData::default();
    let right = Vec3::from(right);
    let up = Vec3::from(up);
    for particle in particles {
        let layer = particle.kind.layer(textures);
        let centre = Vec3::from(particle.position);
        let half = particle.size * 0.5;
        let corners = [
            centre - right * half - up * half,
            centre + right * half - up * half,
            centre + right * half + up * half,
            centre - right * half + up * half,
        ];
        let positions: [[f32; 3]; 4] = [
            corners[0].to_array(),
            corners[1].to_array(),
            corners[2].to_array(),
            corners[3].to_array(),
        ];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for reverse in [false, true] {
            let base = mesh.vertices.len() as u32;
            for index in 0..4 {
                mesh.vertices.push(Vertex {
                    position: positions[index],
                    uv: uvs[index],
                    shade: 1.0,
                    layer,
                    anim: NO_ANIMATION,
                    light: 1.0,
                });
            }
            if reverse {
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 2,
                    base + 1,
                    base,
                    base + 3,
                    base + 2,
                ]);
            } else {
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

fn append_quad(
    mesh: &mut EntityMesh,
    centre: [f32; 3],
    half: [f32; 2],
    right: [f32; 3],
    up: [f32; 3],
    color: [f32; 4],
) {
    let r = [right[0] * half[0], right[1] * half[0], right[2] * half[0]];
    let u = [up[0] * half[1], up[1] * half[1], up[2] * half[1]];
    // Counter-clockwise from the camera, because the entity pipeline culls
    // back faces.
    let corners = [
        [
            centre[0] - r[0] - u[0],
            centre[1] - r[1] - u[1],
            centre[2] - r[2] - u[2],
        ],
        [
            centre[0] + r[0] - u[0],
            centre[1] + r[1] - u[1],
            centre[2] + r[2] - u[2],
        ],
        [
            centre[0] + r[0] + u[0],
            centre[1] + r[1] + u[1],
            centre[2] + r[2] + u[2],
        ],
        [
            centre[0] - r[0] + u[0],
            centre[1] - r[1] + u[1],
            centre[2] - r[2] + u[2],
        ],
    ];
    let base = mesh.vertices.len() as u32;
    for position in corners {
        mesh.vertices.push(EntityVertex {
            position,
            uv: [-1.0, -1.0],
            shade: 1.0,
            color,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Vanilla `EntityNoteFX`: the note's hue walks the rainbow as its pitch rises.
pub fn note_color(pitch: f32) -> [f32; 3] {
    let tau = std::f32::consts::TAU;
    [
        (pitch * tau).sin() * 0.65 + 0.35,
        ((pitch + 1.0 / 3.0) * tau).sin() * 0.65 + 0.35,
        ((pitch + 2.0 / 3.0) * tau).sin() * 0.65 + 0.35,
    ]
}

fn rand01(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use nephele_assets::{TextureArray, TILE_SIZE};

    fn atlas() -> TextureArray {
        let tile = image::RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, Rgba([90, 140, 60, 255]));
        TextureArray::from_tiles(vec![("stone".into(), tile)])
    }

    #[test]
    fn breaking_a_block_emits_camera_facing_quads() {
        let textures = atlas();
        let mut particles = Particles::new();
        particles.spawn_block(&textures, BlockState::new(1, 0), 4, 64, 8);
        assert_eq!(particles.debris_count(), DEBRIS_COUNT);

        let mesh = particles.build_debris([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0);
        assert_eq!(mesh.indices.len(), DEBRIS_COUNT * 6);
        // Every shard is a 4x4 window of the block tile, not the whole tile.
        for debris in &particles.debris {
            assert!(debris.uv[0] + debris.uv[2] <= 1.0 + 1e-6);
            assert!(debris.uv[1] + debris.uv[3] <= 1.0 + 1e-6);
            assert!(debris.uv[2] < 1.0 && debris.uv[3] < 1.0);
        }
    }

    #[test]
    fn debris_falls_and_eventually_expires() {
        let textures = atlas();
        let mut particles = Particles::new();
        particles.spawn_block(&textures, BlockState::new(1, 0), 0, 100, 0);
        let world = World::new(0);
        let start = particles.debris[0].position[1];
        for _ in 0..10 {
            particles.update(0.05, &world);
        }
        assert!(particles.debris[0].position[1] < start);
        for _ in 0..40 {
            particles.update(0.05, &world);
        }
        assert!(particles.is_empty(), "particles should expire");
    }

    #[test]
    fn a_note_rises_and_dies_within_its_six_ticks() {
        let mut particles = Particles::new();
        particles.spawn_note(2, 70, 3, 12);
        assert_eq!(particles.note_count(), 1);
        let start = particles.notes[0].position[1];
        let world = World::new(0);
        particles.update(0.1, &world);
        assert!(particles.notes[0].position[1] > start);
        for _ in 0..6 {
            particles.update(0.05, &world);
        }
        assert_eq!(particles.note_count(), 0);
    }

    #[test]
    fn note_colour_walks_the_hue_circle() {
        let low = note_color(0.0);
        let mid = note_color(0.75);
        assert!((low[0] - 0.35).abs() < 1e-6);
        assert!((mid[0] + 0.3).abs() < 1e-6);
        assert_ne!(low, mid);
    }

    #[test]
    fn note_quads_mark_themselves_untextured() {
        let mut particles = Particles::new();
        particles.spawn_note(0, 64, 0, 3);
        let mut mesh = EntityMesh::default();
        particles.append_notes(&mut mesh, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0);
        // A head and a stem per note, four vertices each.
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
        assert!(mesh.vertices.iter().all(|vertex| vertex.uv == [-1.0, -1.0]));
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color[3] > 0.0 && vertex.color[3] <= 1.0));
    }

    fn sprite_atlas() -> TextureArray {
        let tile = image::RgbaImage::from_pixel(TILE_SIZE, TILE_SIZE, Rgba([90, 140, 60, 255]));
        let mut tiles: Vec<(String, image::RgbaImage)> = vec![("stone".into(), tile)];
        for (name, image) in nephele_assets::particle_textures::layers() {
            tiles.push((name.into(), image));
        }
        TextureArray::from_tiles(tiles)
    }

    #[test]
    fn every_sprite_kind_samples_its_own_generated_layer() {
        let textures = sprite_atlas();
        let missing = textures.slot_or_missing(MISSING_TEXTURE);
        let kinds = [
            ParticleKind::Smoke,
            ParticleKind::Flame,
            ParticleKind::Crit,
            ParticleKind::Portal,
            ParticleKind::Splash,
            ParticleKind::Heart,
        ];
        let mut seen = Vec::new();
        for kind in kinds {
            let layer = kind.layer(&textures);
            assert_ne!(layer, missing, "{kind:?} fell back to the missing tile");
            assert!(!seen.contains(&layer), "{kind:?} shares a layer");
            seen.push(layer);
        }
    }

    #[test]
    fn cracks_reuse_the_block_and_item_layers() {
        let textures = sprite_atlas();
        assert_eq!(
            ParticleKind::BlockBreak(BlockState::new(1, 0)).layer(&textures),
            textures.slot("stone").expect("stone layer")
        );
        // No item layers in this array, so an item crack must degrade to the
        // missing tile rather than panicking or sampling layer 0.
        assert_eq!(
            ParticleKind::ItemBreak { id: 265, meta: 0 }.layer(&textures),
            textures.slot_or_missing(MISSING_TEXTURE)
        );
    }

    #[test]
    fn sprite_quads_are_double_sided() {
        let textures = sprite_atlas();
        let draw = ParticleDraw {
            position: [0.0, 64.0, 0.0],
            size: 0.2,
            kind: ParticleKind::Heart,
        };
        let mesh = build_particle_mesh(&[draw], &textures, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        // Two triangles per side, two sides, so the quad shows from behind too.
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
        let layer = textures.slot_or_missing(nephele_assets::particle_textures::HEART);
        assert!(mesh.vertices.iter().all(|vertex| vertex.layer == layer));
    }

    #[test]
    fn sprites_fall_and_expire() {
        let mut particles = Particles::new();
        particles.spawn_kind(ParticleKind::Flame, [0.0, 64.0, 0.0], [0.0; 3], 0.0, 4);
        assert_eq!(particles.sprite_count(), 4);
        let start = particles.sprites[0].position[1];
        let world = World::new(0);
        particles.update(0.05, &world);
        assert!(particles.sprites[0].position[1] < start);
        for _ in 0..40 {
            particles.update(0.05, &world);
        }
        assert_eq!(particles.sprite_count(), 0);
        assert!(particles.is_empty(), "sprites should expire");
    }

    #[test]
    fn a_particle_burst_is_capped_and_never_negative() {
        let mut particles = Particles::new();
        particles.spawn_kind(ParticleKind::Smoke, [0.0; 3], [1.0; 3], 1.0, 10_000);
        assert_eq!(particles.sprite_count(), 128);
        particles.spawn_kind(ParticleKind::Smoke, [0.0; 3], [1.0; 3], 1.0, -5);
        assert_eq!(particles.sprite_count(), 128);
    }
}
