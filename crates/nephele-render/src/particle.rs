//! Camera-facing particle quads.
//!
//! Particles use the terrain pipeline and its block texture array, so they need
//! no extra GPU state. Block-break particles sample the broken block's side
//! texture; the rest use the generated particle sprites.

use glam::Vec3;
use nephele_assets::{face_textures, item_texture_name, particle_textures, TextureArray};
use nephele_world::BlockState;

use crate::mesh::{MeshData, Vertex};

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
                .map(|faces| textures.layer_or_missing(faces.side))
                .unwrap_or_else(|| textures.layer_or_missing("stone")),
            ParticleKind::ItemBreak { id, meta } => item_texture_name(id, meta)
                .map(|name| textures.layer_or_missing(name))
                .unwrap_or_else(|| textures.layer_or_missing("missing")),
            ParticleKind::Smoke => textures.layer_or_missing(particle_textures::SMOKE),
            ParticleKind::Flame => textures.layer_or_missing(particle_textures::FLAME),
            ParticleKind::Crit => textures.layer_or_missing(particle_textures::CRIT),
            ParticleKind::Portal => textures.layer_or_missing(particle_textures::PORTAL),
            ParticleKind::Splash => textures.layer_or_missing(particle_textures::SPLASH),
            ParticleKind::Heart => textures.layer_or_missing(particle_textures::HEART),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleDraw {
    pub position: [f32; 3],
    /// Edge length in blocks.
    pub size: f32,
    pub kind: ParticleKind,
}

/// Build camera-facing quads for every live particle.
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn textures() -> TextureArray {
        TextureArray::from_tiles(vec![
            (
                "stone",
                RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255])),
            ),
            (
                particle_textures::SMOKE,
                RgbaImage::from_pixel(16, 16, image::Rgba([4, 5, 6, 255])),
            ),
        ])
    }

    #[test]
    fn one_particle_is_two_visible_quads() {
        let mesh = build_particle_mesh(
            &[ParticleDraw {
                position: [0.0, 64.0, 0.0],
                size: 0.25,
                kind: ParticleKind::Smoke,
            }],
            &textures(),
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
        let smoke = textures().layer(particle_textures::SMOKE).unwrap();
        assert!(mesh.vertices.iter().all(|vertex| vertex.layer == smoke));
    }

    #[test]
    fn block_break_uses_the_block_side_texture() {
        let mesh = build_particle_mesh(
            &[ParticleDraw {
                position: [0.0; 3],
                size: 0.2,
                kind: ParticleKind::BlockBreak(BlockState::STONE),
            }],
            &textures(),
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        assert_eq!(mesh.indices.len(), 12);
    }
}
