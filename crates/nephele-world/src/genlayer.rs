//! Vanilla 1.8.9's biome layer stack, reimplemented in Rust.
//!
//! The world is not classified into biomes directly: a chain of ~25 layers,
//! each taking the previous layer's coarse grid and refining it, ends in the
//! 1:1 biome map. Every layer draws from its own LCG seeded from the world seed
//! and the coordinates it is asked about, so the result depends only on those
//! inputs — the order chunks are visited in cannot change the world.
//!
//! The stack itself is `GenLayer.initializeAllBiomeGenerators`; see
//! [`BiomeLayers::new`] for the order.

const LCG_MUL: i64 = 6_364_136_223_846_793_005;
const LCG_ADD: i64 = 1_442_695_040_888_963_407;

/// Biome ids, as the game numbers them.
pub mod biome_id {
    pub const OCEAN: i32 = 0;
    pub const PLAINS: i32 = 1;
    pub const DESERT: i32 = 2;
    pub const EXTREME_HILLS: i32 = 3;
    pub const FOREST: i32 = 4;
    pub const TAIGA: i32 = 5;
    pub const SWAMPLAND: i32 = 6;
    pub const RIVER: i32 = 7;
    pub const FROZEN_OCEAN: i32 = 10;
    pub const FROZEN_RIVER: i32 = 11;
    pub const ICE_PLAINS: i32 = 12;
    pub const MUSHROOM_ISLAND: i32 = 14;
    pub const MUSHROOM_ISLAND_SHORE: i32 = 15;
    pub const BEACH: i32 = 16;
    pub const DEEP_OCEAN: i32 = 24;
    pub const STONE_BEACH: i32 = 25;
    pub const COLD_BEACH: i32 = 26;
    pub const MESA: i32 = 37;
    pub const MESA_PLATEAU_F: i32 = 38;
    pub const MESA_PLATEAU: i32 = 39;
    /// Every mutated biome is `base + 128` (`GenLayerRareBiome`, hills).
    pub const MUTATED: i32 = 128;
}

/// The LCG every layer runs, matching `GenLayer`'s three seeds.
#[derive(Debug, Clone)]
pub struct LayerSeed {
    world_gen_seed: i64,
    base_seed: i64,
    chunk_seed: i64,
}

impl LayerSeed {
    fn new(base_seed: i64) -> Self {
        let mut value = base_seed;
        for _ in 0..3 {
            value = value.wrapping_mul(value.wrapping_mul(LCG_MUL).wrapping_add(LCG_ADD));
            value = value.wrapping_add(base_seed);
        }
        Self {
            world_gen_seed: 0,
            base_seed: value,
            chunk_seed: 0,
        }
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.world_gen_seed = seed;
        for _ in 0..3 {
            self.world_gen_seed = self
                .world_gen_seed
                .wrapping_mul(
                    self.world_gen_seed
                        .wrapping_mul(LCG_MUL)
                        .wrapping_add(LCG_ADD),
                )
                .wrapping_add(self.base_seed);
        }
    }

    fn init_chunk(&mut self, x: i64, z: i64) {
        self.chunk_seed = self.world_gen_seed;
        for step in 0..4 {
            self.chunk_seed = self
                .chunk_seed
                .wrapping_mul(self.chunk_seed.wrapping_mul(LCG_MUL).wrapping_add(LCG_ADD))
                .wrapping_add(if step % 2 == 0 { x } else { z });
        }
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        let mut value = ((self.chunk_seed >> 24) % bound as i64) as i32;
        if value < 0 {
            value += bound;
        }
        self.chunk_seed = self
            .chunk_seed
            .wrapping_mul(self.chunk_seed.wrapping_mul(LCG_MUL).wrapping_add(LCG_ADD))
            .wrapping_add(self.world_gen_seed);
        value
    }

    fn select_random(&mut self, values: &[i32]) -> i32 {
        let index = self.next_int(values.len() as i32) as usize;
        values[index]
    }

    /// `selectModeOrRandom`: the repeated value wins, otherwise draw one.
    ///
    /// Written as the reference's branch order, several branches deliberately
    /// returning `a`, because which branch fires decides whether a draw is
    /// consumed at all.
    #[allow(clippy::if_same_then_else)]
    fn select_mode_or_random(&mut self, a: i32, b: i32, c: i32, d: i32) -> i32 {
        if b == c && c == d {
            b
        } else if a == b && a == c {
            a
        } else if a == b && a == d {
            a
        } else if a == c && a == d {
            a
        } else if a == b && c != d {
            a
        } else if a == c && b != d {
            a
        } else if a == d && b != c {
            a
        } else if b == c && a != d {
            b
        } else if b == d && a != c {
            b
        } else if c == d && a != b {
            c
        } else {
            self.select_random(&[a, b, c, d])
        }
    }
}

/// One layer: takes an area of the parent's grid and returns a refined one.
pub trait Layer {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32>;
    fn set_world_seed(&mut self, seed: i64);
}

fn index(x: usize, z: usize, width: usize) -> usize {
    z * width + x
}

/// `GenLayerIsland`: scatter land, guaranteed to include the origin.
pub struct GenLayerIsland {
    seed: LayerSeed,
}

impl GenLayerIsland {
    pub fn new(base_seed: i64) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
        }
    }
}

impl Layer for GenLayerIsland {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                self.seed
                    .init_chunk((area_x + x as i32) as i64, (area_z + z as i32) as i64);
                out[index(x, z, width)] = if self.seed.next_int(10) == 0 { 1 } else { 0 };
            }
        }
        // The origin is always land, so a world always has somewhere to spawn.
        if area_x > -(width as i32) && area_x <= 0 && area_z > -(height as i32) && area_z <= 0 {
            let ox = (-area_x) as usize;
            let oz = (-area_z) as usize;
            if ox < width && oz < height {
                out[index(ox, oz, width)] = 1;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerZoom` (and `FuzzyZoom`, which only changes how ties are broken).
pub struct GenLayerZoom {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
    fuzzy: bool,
}

impl GenLayerZoom {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
            fuzzy: false,
        }
    }

    pub fn fuzzy(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
            fuzzy: true,
        }
    }

    fn zoom_select(&mut self, a: i32, b: i32, c: i32, d: i32) -> i32 {
        if self.fuzzy {
            self.seed.select_random(&[a, b, c, d])
        } else {
            self.seed.select_mode_or_random(a, b, c, d)
        }
    }
}

impl Layer for GenLayerZoom {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_x = area_x >> 1;
        let parent_z = area_z >> 1;
        let parent_w = (width >> 1) + 2;
        let parent_h = (height >> 1) + 2;
        let parent = self.parent.get_ints(parent_x, parent_z, parent_w, parent_h);

        let zoomed_w = (parent_w - 1) << 1;
        let zoomed_h = (parent_h - 1) << 1;
        let mut zoomed = vec![0; zoomed_w * zoomed_h];

        for z in 0..parent_h - 1 {
            let row = (z << 1) * zoomed_w;
            let mut cursor = 0usize;
            let mut top_left = parent[index(0, z, parent_w)];
            let mut bottom_left = parent[index(0, z + 1, parent_w)];
            while cursor < parent_w - 1 {
                self.seed.init_chunk(
                    (cursor as i32 + (parent_x << 1)) as i64,
                    (z as i32 + (parent_z << 1)) as i64,
                );
                let top_right = parent[index(cursor + 1, z, parent_w)];
                let bottom_right = parent[index(cursor + 1, z + 1, parent_w)];

                zoomed[row + cursor * 2] = top_left;
                zoomed[row + cursor * 2 + zoomed_w] =
                    self.seed.select_random(&[top_left, bottom_left]);
                zoomed[row + cursor * 2 + 1] = self.seed.select_random(&[top_left, top_right]);
                zoomed[row + cursor * 2 + 1 + zoomed_w] =
                    self.zoom_select(top_left, top_right, bottom_left, bottom_right);

                top_left = top_right;
                bottom_left = bottom_right;
                cursor += 1;
            }
        }

        let mut out = vec![0; width * height];
        let x_offset = (area_x & 1) as usize;
        let z_offset = (area_z & 1) as usize;
        for z in 0..height {
            let src_row = (z + z_offset) * zoomed_w + x_offset;
            let len = width.min(zoomed_w.saturating_sub(x_offset));
            out[index(0, z, width)..index(0, z, width) + len]
                .copy_from_slice(&zoomed[src_row..src_row + len]);
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerZoom.magnify(seed, parent, count)`.
pub fn magnify(seed: i64, parent: Box<dyn Layer>, count: u32) -> Box<dyn Layer> {
    let mut layer = parent;
    for step in 0..count {
        layer = Box::new(GenLayerZoom::new(seed + step as i64, layer));
    }
    layer
}

/// `GenLayerSmooth`: knock off single-cell protrusions.
pub struct GenLayerSmooth {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerSmooth {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerSmooth {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];

        for z in 0..height {
            for x in 0..width {
                let west = parent[index(x, z + 1, parent_w)];
                let east = parent[index(x + 2, z + 1, parent_w)];
                let north = parent[index(x + 1, z, parent_w)];
                let south = parent[index(x + 1, z + 2, parent_w)];
                let mut centre = parent[index(x + 1, z + 1, parent_w)];

                if west == east && north == south {
                    self.seed
                        .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                    centre = if self.seed.next_int(2) == 0 {
                        west
                    } else {
                        north
                    };
                } else {
                    if west == east {
                        centre = west;
                    }
                    if north == south {
                        centre = north;
                    }
                }
                out[index(x, z, width)] = centre;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerRareBiome`: turns 1 in 57 plains cells into sunflower plains.
pub struct GenLayerRareBiome {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerRareBiome {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerRareBiome {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                let value = parent[index(x + 1, z + 1, parent_w)];
                out[index(x, z, width)] =
                    if self.seed.next_int(57) == 0 && value == biome_id::PLAINS {
                        biome_id::PLAINS + biome_id::MUTATED
                    } else {
                        value
                    };
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerAddSnow`: assigns a temperature band to each land cell.
pub struct GenLayerAddSnow {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerAddSnow {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerAddSnow {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                let value = parent[index(x + 1, z + 1, parent_w)];
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                out[index(x, z, width)] = if value == 0 {
                    0
                } else {
                    let roll = self.seed.next_int(6);
                    if roll == 0 {
                        4
                    } else if roll <= 1 {
                        3
                    } else {
                        1
                    }
                };
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerRemoveTooMuchOcean`: fills a lone ocean cell surrounded by ocean.
pub struct GenLayerRemoveTooMuchOcean {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerRemoveTooMuchOcean {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerRemoveTooMuchOcean {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                let north = parent[index(x + 1, z, parent_w)];
                let east = parent[index(x + 2, z + 1, parent_w)];
                let west = parent[index(x, z + 1, parent_w)];
                let south = parent[index(x + 1, z + 2, parent_w)];
                let centre = parent[index(x + 1, z + 1, parent_w)];
                out[index(x, z, width)] = centre;
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                if centre == 0
                    && north == 0
                    && east == 0
                    && west == 0
                    && south == 0
                    && self.seed.next_int(2) == 0
                {
                    out[index(x, z, width)] = 1;
                }
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerRiverInit`: seeds the river chain with per-cell ids.
pub struct GenLayerRiverInit {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerRiverInit {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerRiverInit {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent = self.parent.get_ints(area_x, area_z, width, height);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                out[index(x, z, width)] = if parent[index(x, z, width)] > 0 {
                    self.seed.next_int(299_999) + 2
                } else {
                    0
                };
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerRiver`: marks river cells from the edge between two river ids.
pub struct GenLayerRiver {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerRiver {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }

    fn fold(value: i32) -> i32 {
        if value >= 2 {
            2 + (value & 1)
        } else {
            value
        }
    }
}

impl Layer for GenLayerRiver {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                let west = Self::fold(parent[index(x, z + 1, parent_w)]);
                let east = Self::fold(parent[index(x + 2, z + 1, parent_w)]);
                let north = Self::fold(parent[index(x + 1, z, parent_w)]);
                let south = Self::fold(parent[index(x + 1, z + 2, parent_w)]);
                let centre = Self::fold(parent[index(x + 1, z + 1, parent_w)]);
                out[index(x, z, width)] =
                    if centre == west && centre == north && centre == east && centre == south {
                        -1
                    } else {
                        biome_id::RIVER
                    };
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerRiverMix`: paints rivers over the biome chain, keeping oceans.
pub struct GenLayerRiverMix {
    seed: LayerSeed,
    biomes: Box<dyn Layer>,
    rivers: Box<dyn Layer>,
}

impl GenLayerRiverMix {
    pub fn new(base_seed: i64, biomes: Box<dyn Layer>, rivers: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            biomes,
            rivers,
        }
    }
}

impl Layer for GenLayerRiverMix {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let biome = self.biomes.get_ints(area_x, area_z, width, height);
        let river = self.rivers.get_ints(area_x, area_z, width, height);
        let mut out = vec![0; width * height];
        for i in 0..width * height {
            if biome[i] != biome_id::OCEAN && biome[i] != biome_id::DEEP_OCEAN {
                if river[i] == biome_id::RIVER {
                    out[i] = if biome[i] == biome_id::ICE_PLAINS {
                        biome_id::FROZEN_RIVER
                    } else if biome[i] != biome_id::MUSHROOM_ISLAND
                        && biome[i] != biome_id::MUSHROOM_ISLAND_SHORE
                    {
                        river[i] & 255
                    } else {
                        biome_id::MUSHROOM_ISLAND_SHORE
                    };
                } else {
                    out[i] = biome[i];
                }
            } else {
                out[i] = biome[i];
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.biomes.set_world_seed(seed);
        self.rivers.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Constant(i32);

    impl Layer for Constant {
        fn get_ints(&mut self, _x: i32, _z: i32, width: usize, height: usize) -> Vec<i32> {
            vec![self.0; width * height]
        }
        fn set_world_seed(&mut self, _seed: i64) {}
    }

    #[test]
    fn island_is_deterministic_and_seeded() {
        let mut a = GenLayerIsland::new(1);
        let mut b = GenLayerIsland::new(1);
        let mut c = GenLayerIsland::new(1);
        // The base seed only enters through the world seed, as in vanilla.
        a.set_world_seed(1);
        b.set_world_seed(1);
        c.set_world_seed(2);
        let area_a = a.get_ints(0, 0, 32, 32);
        let area_b = b.get_ints(0, 0, 32, 32);
        let area_c = c.get_ints(0, 0, 32, 32);
        assert_eq!(area_a, area_b);
        assert_ne!(area_a, area_c);
        assert!(area_a.contains(&1), "some land should exist");
        assert!(area_a.contains(&0), "some sea should exist");
    }

    #[test]
    fn fuzzy_zoom_differs_from_plain_zoom() {
        // They share the parent and the seed but break ties differently, so with
        // a non-uniform input of 0s and 1s the outputs must diverge somewhere.
        let mut plain = GenLayerZoom {
            seed: LayerSeed::new(2000),
            parent: Box::new(GenLayerIsland::new(1)),
            fuzzy: false,
        };
        let mut fuzzy = GenLayerZoom {
            seed: LayerSeed::new(2000),
            parent: Box::new(GenLayerIsland::new(1)),
            fuzzy: true,
        };
        plain.set_world_seed(12345);
        fuzzy.set_world_seed(12345);
        let a = plain.get_ints(0, 0, 64, 64);
        let b = fuzzy.get_ints(0, 0, 64, 64);
        assert_eq!(a.len(), b.len());
        assert!(a != b, "fuzzy zoom should not match plain zoom");
    }

    #[test]
    fn island_always_contains_the_origin() {
        for seed in [1i64, 42, -7, 999_999] {
            let mut layer = GenLayerIsland::new(1);
            layer.set_world_seed(seed);
            // Ask for the area around the origin, which includes cell (0,0).
            let area = layer.get_ints(-4, -4, 8, 8);
            assert_eq!(area[index(4, 4, 8)], 1, "seed {seed} left the origin wet");
        }
    }

    #[test]
    fn zoom_doubles_a_uniform_area() {
        let mut zoom = GenLayerZoom::new(2000, Box::new(Constant(7)));
        let area = zoom.get_ints(0, 0, 16, 16);
        assert_eq!(area.len(), 256);
        assert!(area.iter().all(|v| *v == 7), "uniform input stays uniform");
    }

    #[test]
    fn zoom_offsets_are_odd_and_negative_safe() {
        let mut zoom = GenLayerZoom::new(2001, Box::new(Constant(3)));
        for (x, z) in [(-1, -1), (1, 0), (-17, 23)] {
            let area = zoom.get_ints(x, z, 16, 16);
            assert_eq!(area.len(), 256);
            assert!(area.iter().all(|v| *v == 3));
        }
    }

    #[test]
    fn smooth_keeps_uniform_areas() {
        let mut smooth = GenLayerSmooth::new(1000, Box::new(Constant(5)));
        let area = smooth.get_ints(0, 0, 8, 8);
        assert!(area.iter().all(|v| *v == 5));
    }

    #[test]
    fn river_init_only_touches_land() {
        let mut layer = GenLayerRiverInit::new(100, Box::new(Constant(0)));
        let area = layer.get_ints(0, 0, 8, 8);
        assert!(area.iter().all(|v| *v == 0), "no land, no river ids");
    }

    #[test]
    fn river_fold_maps_high_ids_to_two_or_three() {
        assert_eq!(GenLayerRiver::fold(0), 0);
        assert_eq!(GenLayerRiver::fold(1), 1);
        assert_eq!(GenLayerRiver::fold(2), 2);
        assert_eq!(GenLayerRiver::fold(3), 3);
        assert_eq!(GenLayerRiver::fold(4), 2);
        assert_eq!(GenLayerRiver::fold(5), 3);
        assert_eq!(GenLayerRiver::fold(299_999), 3);
    }

    #[test]
    fn add_snow_never_returns_a_zero_on_land() {
        let mut layer = GenLayerAddSnow::new(2, Box::new(Constant(1)));
        let area = layer.get_ints(0, 0, 16, 16);
        assert!(area.iter().all(|v| *v != 0));
        assert!(area.iter().all(|v| *v == 1 || *v == 3 || *v == 4));
    }

    #[test]
    fn add_snow_keeps_ocean_empty() {
        let mut layer = GenLayerAddSnow::new(2, Box::new(Constant(0)));
        assert!(layer.get_ints(0, 0, 8, 8).iter().all(|v| *v == 0));
    }

    #[test]
    fn rare_biome_only_mutates_plains() {
        let mut layer = GenLayerRareBiome::new(1001, Box::new(Constant(biome_id::DESERT)));
        assert!(
            layer
                .get_ints(0, 0, 64, 64)
                .iter()
                .all(|v| *v == biome_id::DESERT),
            "desert must never become sunflower plains"
        );
    }

    #[test]
    fn layer_sizes_are_honoured() {
        let mut zoom = GenLayerZoom::new(1, Box::new(Constant(1)));
        for (w, h) in [(1usize, 1usize), (16, 16), (3, 7), (33, 5)] {
            assert_eq!(zoom.get_ints(0, 0, w, h).len(), w * h);
        }
    }
}
