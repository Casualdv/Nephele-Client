//! Client-side particle simulation.
//!
//! The server only says "spawn N of type X here"; this module owns their
//! velocity, lifetime and the camera-facing quads handed to the renderer.

use std::f32::consts::TAU;

use nephele_protocol::packets::play::ParticleSpawn;
use nephele_render::{ParticleDraw, ParticleKind};
use nephele_world::BlockState;

#[derive(Debug, Clone, Copy)]
struct Particle {
    position: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    life: f32,
    size: f32,
    kind: ParticleKind,
}

pub struct Particles {
    items: Vec<Particle>,
    seed: u32,
    age_marker: f32,
}

impl Default for Particles {
    fn default() -> Self {
        Self::new()
    }
}

impl Particles {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            seed: 0x9E37_79B9,
            age_marker: 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (u32::MAX >> 8) as f32
    }

    fn random_range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.random()
    }

    pub fn spawn_event(&mut self, event: &ParticleSpawn) {
        let (kind, count, base_size) = match event.particle_id {
            36 => {
                let id = event.arguments.first().copied().unwrap_or(0) as i16;
                let meta = event.arguments.get(1).copied().unwrap_or(0) as i16;
                (ParticleKind::ItemBreak { id, meta }, 8, 0.12)
            }
            37 | 38 => {
                let raw = event.arguments.first().copied().unwrap_or(1) as u16;
                (
                    ParticleKind::BlockBreak(BlockState::from_raw(raw)),
                    12,
                    0.16,
                )
            }
            5 | 31 => (ParticleKind::Splash, 8, 0.12),
            9 | 10 => (ParticleKind::Crit, 6, 0.18),
            11 | 12 | 29 => (ParticleKind::Smoke, 8, 0.18),
            24 => (ParticleKind::Portal, 8, 0.14),
            26 | 27 => (ParticleKind::Flame, 6, 0.16),
            34 => (ParticleKind::Heart, 3, 0.16),
            _ => (ParticleKind::Smoke, 4, 0.15),
        };
        let count = event.count.clamp(0, 128).min(count.max(1));
        for _ in 0..count {
            let position = [
                event.x + self.random_range(-event.offset_x, event.offset_x),
                event.y + self.random_range(-event.offset_y, event.offset_y),
                event.z + self.random_range(-event.offset_z, event.offset_z),
            ];
            let speed = event.speed.max(0.05);
            let velocity = [
                self.random_range(-speed, speed),
                self.random_range(0.0, speed),
                self.random_range(-speed, speed),
            ];
            let life = self.random_range(0.35, 0.9);
            let size = base_size * self.random_range(0.7, 1.3);
            self.items.push(Particle {
                position,
                velocity,
                age: 0.0,
                life,
                size,
                kind,
            });
        }
    }

    pub fn smoke(&mut self, position: [f32; 3], count: i32) {
        self.spawn_event(&ParticleSpawn {
            particle_id: 11,
            long_distance: false,
            x: position[0],
            y: position[1],
            z: position[2],
            offset_x: 0.2,
            offset_y: 0.2,
            offset_z: 0.2,
            speed: 0.04,
            count,
            arguments: Vec::new(),
        });
    }

    pub fn update(&mut self, delta: f32) {
        let delta = delta.clamp(0.0, 0.1);
        for particle in &mut self.items {
            particle.age += delta;
            particle.velocity[1] -= 9.8 * delta;
            particle.position[0] += particle.velocity[0] * delta;
            particle.position[1] += particle.velocity[1] * delta;
            particle.position[2] += particle.velocity[2] * delta;
        }
        self.items
            .retain(|particle| particle.age < particle.life && particle.position[1] > -64.0);
    }

    pub fn draws(&self) -> Vec<ParticleDraw> {
        self.items
            .iter()
            .map(|particle| ParticleDraw {
                position: particle.position,
                size: particle.size * (1.0 - particle.age / particle.life * 0.35),
                kind: particle.kind,
            })
            .collect()
    }

    pub fn ambient_tick(&mut self, delta: f32, position: [f32; 3], night: bool) {
        // A small ambient plume keeps a still night alive even before a mob
        // wanders close enough to make its own sound.
        let interval = if night { 0.35 } else { 1.5 };
        self.age_marker += delta;
        if self.age_marker >= interval {
            self.age_marker = 0.0;
            if night {
                let angle = self.random_range(0.0, TAU);
                let radius = self.random_range(2.0, 10.0);
                let x = position[0] + angle.cos() * radius;
                let z = position[2] + angle.sin() * radius;
                self.smoke([x, position[1] + 0.2, z], 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_break_burst_expires_after_its_lifetime() {
        let mut particles = Particles::new();
        particles.spawn_event(&ParticleSpawn {
            particle_id: 37,
            long_distance: false,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            offset_x: 0.2,
            offset_y: 0.2,
            offset_z: 0.2,
            speed: 0.05,
            count: 4,
            arguments: vec![BlockState::STONE.raw() as i32],
        });
        assert_eq!(particles.draws().len(), 4);
        for _ in 0..20 {
            particles.update(0.1);
        }
        assert!(particles.draws().is_empty());
    }

    #[test]
    fn ambient_night_ticks_create_a_particle() {
        let mut particles = Particles::new();
        particles.ambient_tick(0.5, [0.0, 64.0, 0.0], true);
        assert!(!particles.draws().is_empty());
    }
}
