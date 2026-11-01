//! The vanilla 1.8.9 terrain height field, ported from
//! `ChunkProviderGenerate.func_147423_a` and its block-filling loop.
//!
//! A chunk's blocks are decided by a 5x33x5 lattice of density samples that
//! spans `x*4` to `x*4 + 16` horizontally and `0` to `256` vertically, then
//! trilinearly interpolated down to individual blocks. Every sample is
//!
//! ```text
//! density = denormalizeClamp(lower, upper, main) - falloff
//! ```
//!
//! where `lower` and `upper` are two independent noise fields, `main` slides
//! between them, and `falloff` pushes the surface away from the biome's base
//! height. A block is stone when the interpolated value is positive and water
//! (or air) below [`SEA_LEVEL`] when it is not.
//!
//! The noise generators must be built from one shared [`JavaRandom`] in the
//! same order as vanilla's constructor, because each consumes draws from that
//! stream and the later ones would otherwise be seeded differently.

use crate::biome;
use crate::genlayer::BiomeLayers;
use crate::noise::{JavaRandom, NoiseOctaves, NoisePerlin};

/// `ChunkProviderSettings.Factory.seaLevel`: everything below this is water
/// wherever the density is not positive.
pub const SEA_LEVEL: i32 = 63;

/// Blocks per horizontal lattice cell.
const HORIZONTAL: i32 = 4;
/// Blocks per vertical lattice cell.
const VERTICAL: i32 = 8;
/// Lattice samples per chunk: 5x33x5.
const LATTICE_XZ: usize = 5;
const LATTICE_Y: usize = 33;
/// `5 * 33 * 5`.
const DENSITY_LEN: usize = LATTICE_XZ * LATTICE_Y * LATTICE_XZ;

const COORDINATE_SCALE: f64 = 684.412;
const HEIGHT_SCALE: f64 = 684.412;
const LOWER_LIMIT_SCALE: f64 = 512.0;
const UPPER_LIMIT_SCALE: f64 = 512.0;
const DEPTH_NOISE_SCALE: f64 = 200.0;
const MAIN_NOISE_SCALE_X: f64 = 80.0;
const MAIN_NOISE_SCALE_Y: f64 = 160.0;
const MAIN_NOISE_SCALE_Z: f64 = 80.0;
const BASE_SIZE: f64 = 8.5;
const STRETCH_Y: f64 = 12.0;

/// The terrain noise fields for one world seed.
///
/// Reuse one instance across chunks: construction is expensive and the arrays
/// are recycled, exactly as vanilla does.
pub struct TerrainNoise {
    lower_limit: NoiseOctaves,
    upper_limit: NoiseOctaves,
    main: NoiseOctaves,
    depth: NoiseOctaves,
    /// `NoiseGeneratorPerlin(rand, 4)`, the stone-noise field used by
    /// `replaceBlocksForBiome`. Held as an octave set for now: the two
    /// constructors draw from the shared [`JavaRandom`] in the same pattern,
    /// so this keeps the stream aligned for the generators after it.
    surface: NoisePerlin,
    /// Created only to advance the shared random; vanilla uses it for trees.
    _spawner: NoiseOctaves,
    parabola: [f32; 25],
    density: [f64; DENSITY_LEN],
    depth_noise: [f64; LATTICE_XZ * LATTICE_XZ],
    main_noise: [f64; DENSITY_LEN],
    lower_noise: [f64; DENSITY_LEN],
    upper_noise: [f64; DENSITY_LEN],
}

impl TerrainNoise {
    pub fn new(seed: i64) -> Self {
        let mut random = JavaRandom::new(seed);
        // Order matters: each constructor consumes draws from `random`.
        let lower_limit = NoiseOctaves::new(&mut random, 16);
        let upper_limit = NoiseOctaves::new(&mut random, 16);
        let main = NoiseOctaves::new(&mut random, 8);
        let surface = NoisePerlin::new(&mut random, 4);
        let _forest = NoiseOctaves::new(&mut random, 10);
        let depth = NoiseOctaves::new(&mut random, 16);
        let spawner = NoiseOctaves::new(&mut random, 8);

        let mut parabola = [0.0; 25];
        for i in -2..=2 {
            for j in -2..=2 {
                let squared = (i * i + j * j) as f32;
                parabola[(i + 2 + (j + 2) * 5) as usize] = 10.0 / (squared + 0.2).sqrt();
            }
        }

        Self {
            lower_limit,
            upper_limit,
            main,
            depth,
            surface,
            _spawner: spawner,
            parabola,
            density: [0.0; DENSITY_LEN],
            depth_noise: [0.0; LATTICE_XZ * LATTICE_XZ],
            main_noise: [0.0; DENSITY_LEN],
            lower_noise: [0.0; DENSITY_LEN],
            upper_noise: [0.0; DENSITY_LEN],
        }
    }

    /// Fills the 5x33x5 density lattice for one chunk.
    ///
    /// `biomes` is the 10x10 block-resolution biome sample vanilla asks
    /// `getBiomesForGeneration` for, starting two blocks before the chunk.
    pub fn fill_density(&mut self, chunk_x: i32, chunk_z: i32, biomes: &[i32]) {
        assert_eq!(biomes.len(), 100, "10x10 biome sample");

        let x = chunk_x * HORIZONTAL;
        let z = chunk_z * HORIZONTAL;

        self.depth.generate_2d(
            &mut self.depth_noise,
            x,
            z,
            LATTICE_XZ,
            LATTICE_XZ,
            DEPTH_NOISE_SCALE,
            DEPTH_NOISE_SCALE,
        );
        self.main.generate_octaves(
            &mut self.main_noise,
            x,
            0,
            z,
            LATTICE_XZ,
            LATTICE_Y,
            LATTICE_XZ,
            COORDINATE_SCALE / MAIN_NOISE_SCALE_X,
            HEIGHT_SCALE / MAIN_NOISE_SCALE_Y,
            COORDINATE_SCALE / MAIN_NOISE_SCALE_Z,
        );
        self.lower_limit.generate_octaves(
            &mut self.lower_noise,
            x,
            0,
            z,
            LATTICE_XZ,
            LATTICE_Y,
            LATTICE_XZ,
            COORDINATE_SCALE,
            HEIGHT_SCALE,
            COORDINATE_SCALE,
        );
        self.upper_limit.generate_octaves(
            &mut self.upper_noise,
            x,
            0,
            z,
            LATTICE_XZ,
            LATTICE_Y,
            LATTICE_XZ,
            COORDINATE_SCALE,
            HEIGHT_SCALE,
            COORDINATE_SCALE,
        );

        let mut index = 0;
        let mut depth_index = 0;

        for k in 0..LATTICE_XZ {
            for l in 0..LATTICE_XZ {
                let centre = biomes[k + 2 + (l + 2) * 10];
                let (centre_root, _) = biome::height_params(centre);

                let mut depth_total = 0.0f32;
                let mut scale_total = 0.0f32;
                let mut weight_total = 0.0f32;

                for j1 in -2i32..=2 {
                    for k1 in -2i32..=2 {
                        let neighbour =
                            biomes[(k as i32 + j1 + 2 + (l as i32 + k1 + 2) * 10) as usize];
                        let (root, variation) = biome::height_params(neighbour);
                        let mut weight =
                            self.parabola[(j1 + 2 + (k1 + 2) * 5) as usize] / (root + 2.0);
                        if root > centre_root {
                            weight /= 2.0;
                        }
                        depth_total += variation * weight;
                        scale_total += root * weight;
                        weight_total += weight;
                    }
                }

                let depth_average = depth_total / weight_total * 0.9 + 0.1;
                let scale_average = (scale_total / weight_total * 4.0 - 1.0) / 8.0;

                let mut wobble = self.depth_noise[depth_index] / 8000.0;
                depth_index += 1;
                if wobble < 0.0 {
                    wobble = -wobble * 0.3;
                }
                wobble = wobble * 3.0 - 2.0;
                if wobble < 0.0 {
                    wobble /= 2.0;
                    if wobble < -1.0 {
                        wobble = -1.0;
                    }
                    wobble /= 1.4;
                    wobble /= 2.0;
                } else {
                    if wobble > 1.0 {
                        wobble = 1.0;
                    }
                    wobble /= 8.0;
                }

                let mut base = f64::from(scale_average);
                let spread = f64::from(depth_average);
                base += wobble * 0.2;
                base = base * BASE_SIZE / 8.0;
                let target = BASE_SIZE + base * 4.0;

                for level in 0..LATTICE_Y {
                    let mut falloff = (level as f64 - target) * STRETCH_Y * 128.0 / 256.0 / spread;
                    if falloff < 0.0 {
                        falloff *= 4.0;
                    }

                    let lower = self.lower_noise[index] / LOWER_LIMIT_SCALE;
                    let upper = self.upper_noise[index] / UPPER_LIMIT_SCALE;
                    let blend = (self.main_noise[index] / 10.0 + 1.0) / 2.0;
                    let mut value = denormalize_clamp(lower, upper, blend) - falloff;

                    if level > 29 {
                        let taper = ((level - 29) as f32 / 3.0) as f64;
                        value = value * (1.0 - taper) - 10.0 * taper;
                    }

                    self.density[index] = value;
                    index += 1;
                }
            }
        }
    }

    /// The same density lattice, interpolated to blocks, reduced to the top
    /// solid `y` of each column. `-1` means the column is empty all the way
    /// down (open ocean).
    pub fn surface_heights(&self) -> [i32; 256] {
        let mut surface = [-1i32; 256];

        for cell_x in 0..4usize {
            let base_x = cell_x * LATTICE_XZ;
            let next_x = (cell_x + 1) * LATTICE_XZ;

            for cell_z in 0..4usize {
                let a = (base_x + cell_z) * LATTICE_Y;
                let b = (base_x + cell_z + 1) * LATTICE_Y;
                let c = (next_x + cell_z) * LATTICE_Y;
                let d = (next_x + cell_z + 1) * LATTICE_Y;

                for step in 0..32usize {
                    let mut near_a = self.density[a + step];
                    let mut near_b = self.density[b + step];
                    let mut far_a = self.density[c + step];
                    let mut far_b = self.density[d + step];

                    let delta_a = (self.density[a + step + 1] - near_a) * 0.125;
                    let delta_b = (self.density[b + step + 1] - near_b) * 0.125;
                    let delta_c = (self.density[c + step + 1] - far_a) * 0.125;
                    let delta_d = (self.density[d + step + 1] - far_b) * 0.125;

                    for sub_y in 0..VERTICAL {
                        let mut across_a = near_a;
                        let mut across_b = near_b;
                        let slope_a = (far_a - near_a) * 0.25;
                        let slope_b = (far_b - near_b) * 0.25;

                        for sub_x in 0..4usize {
                            let between = (across_b - across_a) * 0.25;
                            let mut value = across_a - between;

                            for sub_z in 0..4usize {
                                value += between;
                                if value > 0.0 {
                                    let block_x = cell_x * 4 + sub_x;
                                    let block_y = step as i32 * VERTICAL + sub_y;
                                    let block_z = cell_z * 4 + sub_z;
                                    let column = block_z * 16 + block_x;
                                    if block_y > surface[column] {
                                        surface[column] = block_y;
                                    }
                                }
                            }

                            across_a += slope_a;
                            across_b += slope_b;
                        }

                        near_a += delta_a;
                        near_b += delta_b;
                        far_a += delta_c;
                        far_b += delta_d;
                    }
                }
            }
        }

        surface
    }

    /// `replaceBlocksForBiome`'s stone-noise field: one value per block of a
    /// chunk, used to vary how deep the surface layer sits.
    pub fn stone_noise(&self, chunk_x: i32, chunk_z: i32) -> [f64; 256] {
        let mut noise = [0.0; 256];
        let scale = 0.03125 * 2.0;
        self.surface.populate(
            &mut noise,
            f64::from(chunk_x * 16),
            f64::from(chunk_z * 16),
            16,
            16,
            scale,
            scale,
        );
        noise
    }

    /// Convenience: the 10x10 biome sample, the lattice, and the surface.
    pub fn chunk_surface(
        &mut self,
        layers: &mut BiomeLayers,
        chunk_x: i32,
        chunk_z: i32,
    ) -> [i32; 256] {
        let biomes = layers.terrain_biomes(chunk_x * 4 - 2, chunk_z * 4 - 2, 10, 10);
        self.chunk_surface_with_biomes(chunk_x, chunk_z, &biomes)
    }

    /// The same, with a caller-supplied 10x10 biome sample.
    pub fn chunk_surface_with_biomes(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        biomes: &[i32],
    ) -> [i32; 256] {
        self.fill_density(chunk_x, chunk_z, biomes);
        self.surface_heights()
    }
}

/// `MathHelper.denormalizeClamp`.
fn denormalize_clamp(lower: f64, upper: f64, blend: f64) -> f64 {
    if blend < 0.0 {
        lower
    } else if blend > 1.0 {
        upper
    } else {
        lower + (upper - lower) * blend
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormalize_clamp_picks_the_ends_outside_the_unit_range() {
        assert_eq!(denormalize_clamp(3.0, 7.0, -0.5), 3.0);
        assert_eq!(denormalize_clamp(3.0, 7.0, 1.5), 7.0);
        assert_eq!(denormalize_clamp(3.0, 7.0, 0.5), 5.0);
    }

    #[test]
    fn height_presets_cover_the_vanilla_biomes() {
        assert_eq!(biome::height_params(0), (-1.0, 0.1));
        assert_eq!(biome::height_params(3), (1.0, 0.5));
        assert_eq!(biome::height_params(24), (-1.8, 0.1));
        assert_eq!(biome::height_params(36), (1.5, 0.025));
        // Mutated biomes are their base plus (0.1, 0.2).
        assert_eq!(biome::height_params(128 + 3), (1.1, 0.7));
        let mutated = biome::height_params(128 + 24);
        assert!((mutated.0 - -1.7).abs() < 1e-6, "{mutated:?}");
        assert_eq!(mutated.1, 0.3);
    }

    /// The height field with every biome forced to plains, checked against the
    /// oracle's own transcription of `func_147423_a` run on the real noise
    /// classes (`Nephele-private/oracle`, `Terrain.java`). This pins the noise
    /// pipeline and the interpolation on their own, with no biome data in the
    /// way.
    /// The stone-noise field, checked against the oracle's real
    /// `NoiseGeneratorPerlin` for the same seed and chunk.
    #[test]
    fn stone_noise_matches_the_java_reference() {
        let expected = [
            -1.700268253114639,
            -1.7344816783366233,
            -1.7267996354752175,
            -1.690807821230924,
            -1.6484226117240335,
            -1.6243574595299737,
            -1.638910039133802,
            -1.70094759813542,
        ];
        let noise = TerrainNoise::new(12345);
        let stone = noise.stone_noise(-28, 4);
        for (index, want) in expected.iter().enumerate() {
            assert!(
                (stone[index] - want).abs() < 1e-12,
                "column {index}: {} vs {want}",
                stone[index]
            );
        }
    }

    #[test]
    fn a_flat_biome_chunk_matches_the_java_reference() {
        let expected: [i32; 256] = [
            70, 70, 70, 70, 71, 71, 71, 71, 71, 72, 72, 73, 73, 73, 72, 72, //
            70, 70, 70, 71, 71, 71, 71, 71, 71, 72, 72, 72, 73, 73, 72, 72, //
            70, 70, 70, 71, 71, 71, 71, 71, 71, 72, 72, 72, 73, 73, 72, 72, //
            70, 70, 70, 71, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 73, 73, //
            70, 70, 71, 71, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 73, 73, //
            70, 70, 71, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 73, 73, //
            70, 70, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 72, 73, 73, //
            70, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 72, 72, 73, 73, //
            70, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 72, 72, 72, 72, 73, //
            70, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 72, 72, 72, 72, 72, //
            70, 71, 71, 71, 71, 72, 72, 72, 73, 73, 72, 72, 72, 72, 71, 71, //
            70, 71, 71, 71, 71, 71, 72, 72, 73, 73, 72, 72, 72, 71, 71, 71, //
            70, 71, 71, 71, 71, 71, 72, 72, 73, 73, 72, 72, 72, 71, 71, 70, //
            70, 70, 70, 71, 71, 71, 71, 72, 72, 72, 72, 72, 71, 71, 71, 70, //
            70, 70, 70, 70, 71, 71, 71, 71, 71, 71, 71, 71, 71, 71, 71, 70, //
            69, 70, 70, 70, 70, 71, 71, 71, 71, 71, 71, 71, 71, 71, 71, 70, //
        ];

        let mut noise = TerrainNoise::new(12345);
        let plains = [1i32; 100];
        let surface = noise.chunk_surface_with_biomes(-28, 4, &plains);
        assert_eq!(surface, expected);
    }

    #[test]
    fn the_surface_is_deterministic_for_a_seed() {
        let mut layers = BiomeLayers::new(12345);
        let mut noise = TerrainNoise::new(12345);
        let first = noise.chunk_surface(&mut layers, -28, 4);
        let second = noise.chunk_surface(&mut layers, -28, 4);
        assert_eq!(first, second);
    }

    #[test]
    fn terrain_surface_matches_a_world_the_real_server_generated() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        if !dir.is_dir() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let mut layers = BiomeLayers::new(12345);
        let mut noise = TerrainNoise::new(12345);

        let mut compared = 0usize;
        let mut mismatched = 0usize;
        let mut chunks = 0usize;
        let mut example = None;

        for name in ["r.-1.-1.mca", "r.-1.0.mca"] {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let region = RegionFile::open(&path).expect("region opens");
            for lz in 0..32 {
                for lx in 0..32 {
                    let Some(bytes) = region.chunk_nbt(lx, lz).ok().flatten() else {
                        continue;
                    };
                    let Ok(stored) = parse_chunk(&bytes) else {
                        continue;
                    };

                    // Only chunks whose per-block biome map we already
                    // reproduce exactly. The terrain samples biomes, so a
                    // chunk with a wrong biome would blame the height field
                    // for someone else's bug.
                    let biomes = layers.chunk_biomes(stored.x, stored.z);
                    if biomes.len() != stored.biomes.len()
                        || biomes
                            .iter()
                            .zip(&stored.biomes)
                            .any(|(ours, real)| *ours != i32::from(*real))
                    {
                        continue;
                    }
                    chunks += 1;

                    let ours = noise.chunk_surface(&mut layers, stored.x, stored.z);

                    for z in 0..16usize {
                        for x in 0..16usize {
                            let mut top = None;
                            for y in (0..256).rev() {
                                if stored.state(x, y, z) >> 4 != 0 {
                                    top = Some(y);
                                    break;
                                }
                            }
                            let Some(top) = top else { continue };
                            // Only grass-topped columns: water, sand, snow and
                            // trees all sit on the surface and would be a
                            // different question.
                            if stored.state(x, top, z) >> 4 != 2 {
                                continue;
                            }

                            compared += 1;
                            let predicted = ours[z * 16 + x];
                            if predicted != top as i32 {
                                if example.is_none() {
                                    example = Some((
                                        stored.x * 16 + x as i32,
                                        top as i32,
                                        predicted,
                                        stored.z * 16 + z as i32,
                                    ));
                                }
                                mismatched += 1;
                            }
                        }
                    }
                }
            }
        }

        assert!(
            compared > 1000 && chunks > 20,
            "only {compared} columns in {chunks} chunks compared"
        );
        let percent = mismatched as f64 / compared as f64 * 100.0;
        assert!(
            percent < 6.0,
            "{mismatched} of {compared} grass columns differ ({percent:.1}%) across \
             {chunks} chunks, first (block x, world y, ours, block z) = {example:?}"
        );
    }

    #[test]
    fn the_surface_stays_inside_the_world() {
        let mut layers = BiomeLayers::new(12345);
        let mut noise = TerrainNoise::new(12345);
        for (cx, cz) in [(-28, 4), (0, 0), (-3, -7), (12, -9)] {
            let surface = noise.chunk_surface(&mut layers, cx, cz);
            for column in surface {
                assert!(
                    (-1..256).contains(&column),
                    "column top {column} out of range"
                );
            }
        }
    }
}
