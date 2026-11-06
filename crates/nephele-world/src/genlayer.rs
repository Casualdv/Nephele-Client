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
                    ((cursor as i32 + parent_x) << 1) as i64,
                    ((z as i32 + parent_z) << 1) as i64,
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

use std::cell::RefCell;
use std::rc::Rc;

use crate::biome;

/// A layer shared by two parents, as vanilla shares `genlayer4` and the river
/// chain between the biome and river branches.
#[derive(Clone)]
pub struct SharedLayer(Rc<RefCell<Box<dyn Layer>>>);

impl SharedLayer {
    pub fn new(inner: Box<dyn Layer>) -> Self {
        Self(Rc::new(RefCell::new(inner)))
    }

    fn boxed(&self) -> Box<dyn Layer> {
        Box::new(self.clone())
    }
}

impl Layer for SharedLayer {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        self.0.borrow_mut().get_ints(area_x, area_z, width, height)
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.0.borrow_mut().set_world_seed(seed);
    }
}

/// `GenLayerAddIsland`: grow land into neighbouring ocean cells.
pub struct GenLayerAddIsland {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerAddIsland {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerAddIsland {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];

        for z in 0..height {
            for x in 0..width {
                let northwest = parent[index(x, z, parent_w)];
                let northeast = parent[index(x + 2, z, parent_w)];
                let southwest = parent[index(x, z + 2, parent_w)];
                let southeast = parent[index(x + 2, z + 2, parent_w)];
                let centre = parent[index(x + 1, z + 1, parent_w)];
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);

                let dry_neighbours =
                    northwest == 0 && northeast == 0 && southwest == 0 && southeast == 0;
                let value = if centre != 0 || dry_neighbours {
                    let touches_water =
                        northwest == 0 || northeast == 0 || southwest == 0 || southeast == 0;
                    if centre > 0 && touches_water {
                        if self.seed.next_int(5) == 0 {
                            if centre == 4 {
                                4
                            } else {
                                0
                            }
                        } else {
                            centre
                        }
                    } else {
                        centre
                    }
                } else {
                    let mut choice = 1;
                    let mut bound = 1;
                    if northwest != 0 {
                        let draw = self.seed.next_int(bound);
                        bound += 1;
                        if draw == 0 {
                            choice = northwest;
                        }
                    }
                    if northeast != 0 {
                        let draw = self.seed.next_int(bound);
                        bound += 1;
                        if draw == 0 {
                            choice = northeast;
                        }
                    }
                    if southwest != 0 {
                        let draw = self.seed.next_int(bound);
                        bound += 1;
                        if draw == 0 {
                            choice = southwest;
                        }
                    }
                    if southeast != 0 {
                        let draw = self.seed.next_int(bound);
                        if draw == 0 {
                            choice = southeast;
                        }
                    }
                    if self.seed.next_int(3) == 0 {
                        choice
                    } else if choice == 4 {
                        4
                    } else {
                        0
                    }
                };
                out[index(x, z, width)] = value;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerAddMushroomIsland`: a 1-in-100 mushroom island in open sea.
pub struct GenLayerAddMushroomIsland {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerAddMushroomIsland {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerAddMushroomIsland {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                let northwest = parent[index(x, z, parent_w)];
                let northeast = parent[index(x + 2, z, parent_w)];
                let southwest = parent[index(x, z + 2, parent_w)];
                let southeast = parent[index(x + 2, z + 2, parent_w)];
                let centre = parent[index(x + 1, z + 1, parent_w)];
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                out[index(x, z, width)] = if centre == 0
                    && northwest == 0
                    && northeast == 0
                    && southwest == 0
                    && southeast == 0
                    && self.seed.next_int(100) == 0
                {
                    biome_id::MUSHROOM_ISLAND
                } else {
                    centre
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

/// `GenLayerDeepOcean`: cells deep inside an ocean become deep ocean.
pub struct GenLayerDeepOcean {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerDeepOcean {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerDeepOcean {
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
                let oceans = [north, east, west, south]
                    .iter()
                    .filter(|value| **value == 0)
                    .count();
                out[index(x, z, width)] = if centre == 0 && oceans > 3 {
                    biome_id::DEEP_OCEAN
                } else {
                    centre
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    CoolWarm,
    HeatIce,
    Special,
}

/// `GenLayerEdge`: inserts a transition band between temperature extremes.
pub struct GenLayerEdge {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
    mode: EdgeMode,
}

impl GenLayerEdge {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>, mode: EdgeMode) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
            mode,
        }
    }
}

impl Layer for GenLayerEdge {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        match self.mode {
            EdgeMode::CoolWarm | EdgeMode::HeatIce => {
                let parent_w = width + 2;
                let parent = self
                    .parent
                    .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
                let mut out = vec![0; width * height];

                let (from, triggers) = if self.mode == EdgeMode::CoolWarm {
                    (1, [3, 4])
                } else {
                    (4, [2, 1])
                };

                for z in 0..height {
                    for x in 0..width {
                        if self.mode == EdgeMode::CoolWarm {
                            self.seed
                                .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                        }
                        let mut centre = parent[index(x + 1, z + 1, parent_w)];
                        if centre == from {
                            let north = parent[index(x + 1, z, parent_w)];
                            let east = parent[index(x + 2, z + 1, parent_w)];
                            let west = parent[index(x, z + 1, parent_w)];
                            let south = parent[index(x + 1, z + 2, parent_w)];
                            if [north, east, west, south]
                                .iter()
                                .any(|value| triggers.contains(value))
                            {
                                centre = if from == 1 { 2 } else { 3 };
                            }
                        }
                        out[index(x, z, width)] = centre;
                    }
                }
                out
            }
            EdgeMode::Special => {
                let parent = self.parent.get_ints(area_x, area_z, width, height);
                let mut out = vec![0; width * height];
                for z in 0..height {
                    for x in 0..width {
                        self.seed
                            .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                        let mut value = parent[index(x, z, width)];
                        if value != 0 && self.seed.next_int(13) == 0 {
                            value |= ((1 + self.seed.next_int(15)) << 8) & 3840;
                        }
                        out[index(x, z, width)] = value;
                    }
                }
                out
            }
        }
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerBiome`: turns the temperature/rainfall bands into real biomes.
pub struct GenLayerBiome {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

const WARM: [i32; 6] = [
    biome_id::DESERT,
    biome_id::DESERT,
    biome_id::DESERT,
    35,
    35,
    biome_id::PLAINS,
];
const MEDIUM: [i32; 6] = [
    4,
    29,
    biome_id::EXTREME_HILLS,
    biome_id::PLAINS,
    27,
    biome_id::SWAMPLAND,
];
const COOL: [i32; 4] = [
    4,
    biome_id::EXTREME_HILLS,
    biome_id::TAIGA,
    biome_id::PLAINS,
];
const ICY: [i32; 4] = [
    biome_id::ICE_PLAINS,
    biome_id::ICE_PLAINS,
    biome_id::ICE_PLAINS,
    30,
];

impl GenLayerBiome {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerBiome {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent = self.parent.get_ints(area_x, area_z, width, height);
        let mut out = vec![0; width * height];
        for z in 0..height {
            for x in 0..width {
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                let raw = parent[index(x, z, width)];
                let special = (raw & 3840) >> 8;
                let band = raw & !3840;
                let value = if biome::is_oceanic(band) || band == biome_id::MUSHROOM_ISLAND {
                    band
                } else if band == 1 {
                    if special > 0 {
                        if self.seed.next_int(3) == 0 {
                            biome_id::MESA_PLATEAU
                        } else {
                            biome_id::MESA_PLATEAU_F
                        }
                    } else {
                        let choice = self.seed.next_int(WARM.len() as i32) as usize;
                        WARM[choice]
                    }
                } else if band == 2 {
                    if special > 0 {
                        21
                    } else {
                        let choice = self.seed.next_int(MEDIUM.len() as i32) as usize;
                        MEDIUM[choice]
                    }
                } else if band == 3 {
                    if special > 0 {
                        32
                    } else {
                        let choice = self.seed.next_int(COOL.len() as i32) as usize;
                        COOL[choice]
                    }
                } else if band == 4 {
                    let choice = self.seed.next_int(ICY.len() as i32) as usize;
                    ICY[choice]
                } else {
                    biome_id::MUSHROOM_ISLAND
                };
                out[index(x, z, width)] = value;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerBiomeEdge`: softens hard jumps between unrelated biomes.
pub struct GenLayerBiomeEdge {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerBiomeEdge {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }

    fn neighbours(parent: &[i32], parent_w: usize, x: usize, z: usize) -> [i32; 4] {
        [
            parent[index(x + 1, z, parent_w)],
            parent[index(x + 2, z + 1, parent_w)],
            parent[index(x, z + 1, parent_w)],
            parent[index(x + 1, z + 2, parent_w)],
        ]
    }

    fn replace_if_necessary(
        parent: &[i32],
        parent_w: usize,
        x: usize,
        z: usize,
        centre: i32,
        from: i32,
        to: i32,
    ) -> Option<i32> {
        if !biome::equal_or_mesa_plateau(centre, from) {
            return None;
        }
        let neighbours = Self::neighbours(parent, parent_w, x, z);
        Some(
            if neighbours
                .iter()
                .all(|value| biome::can_be_neighbors(*value, to))
            {
                from
            } else {
                to
            },
        )
    }

    fn replace(
        parent: &[i32],
        parent_w: usize,
        x: usize,
        z: usize,
        centre: i32,
        from: i32,
        to: i32,
    ) -> Option<i32> {
        if centre != from {
            return None;
        }
        let neighbours = Self::neighbours(parent, parent_w, x, z);
        Some(
            if neighbours
                .iter()
                .all(|value| biome::equal_or_mesa_plateau(*value, to))
            {
                from
            } else {
                to
            },
        )
    }
}

impl Layer for GenLayerBiomeEdge {
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
                let centre = parent[index(x + 1, z + 1, parent_w)];

                let replaced = Self::replace_if_necessary(&parent, parent_w, x, z, centre, 3, 20)
                    .or_else(|| Self::replace(&parent, parent_w, x, z, centre, 38, 37))
                    .or_else(|| Self::replace(&parent, parent_w, x, z, centre, 39, 37))
                    .or_else(|| Self::replace(&parent, parent_w, x, z, centre, 32, 5));

                out[index(x, z, width)] = match replaced {
                    Some(value) => value,
                    None => {
                        let neighbours = Self::neighbours(&parent, parent_w, x, z);
                        if centre == biome_id::DESERT {
                            if neighbours.contains(&biome_id::ICE_PLAINS) {
                                34
                            } else {
                                centre
                            }
                        } else if centre == biome_id::SWAMPLAND {
                            let hostile =
                                neighbours.iter().any(|value| matches!(*value, 2 | 30 | 12));
                            if hostile {
                                biome_id::PLAINS
                            } else if neighbours.contains(&21) {
                                23
                            } else {
                                centre
                            }
                        } else {
                            centre
                        }
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

/// `GenLayerHills`: promotes some cells to their hill variant.
pub struct GenLayerHills {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
    hill_noise: Box<dyn Layer>,
}

impl GenLayerHills {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>, hill_noise: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
            hill_noise,
        }
    }

    fn hill_variant(&mut self, value: i32) -> i32 {
        match value {
            biome_id::DESERT => 17,
            4 => 18,
            27 => 28,
            29 => biome_id::PLAINS,
            biome_id::TAIGA => 19,
            32 => 33,
            30 => 31,
            biome_id::PLAINS => {
                if self.seed.next_int(3) == 0 {
                    18
                } else {
                    4
                }
            }
            biome_id::ICE_PLAINS => 13,
            21 => 22,
            biome_id::OCEAN => biome_id::DEEP_OCEAN,
            biome_id::EXTREME_HILLS => 34,
            35 => 36,
            other if biome::equal_or_mesa_plateau(other, biome_id::MESA_PLATEAU_F) => {
                biome_id::MESA
            }
            biome_id::DEEP_OCEAN if self.seed.next_int(3) == 0 => {
                if self.seed.next_int(2) == 0 {
                    biome_id::PLAINS
                } else {
                    4
                }
            }
            other => other,
        }
    }
}

impl Layer for GenLayerHills {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let parent_w = width + 2;
        let parent = self
            .parent
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let variance = self
            .hill_noise
            .get_ints(area_x - 1, area_z - 1, parent_w, height + 2);
        let mut out = vec![0; width * height];

        for z in 0..height {
            for x in 0..width {
                self.seed
                    .init_chunk((x as i32 + area_x) as i64, (z as i32 + area_z) as i64);
                let centre = parent[index(x + 1, z + 1, parent_w)];
                let noise = variance[index(x + 1, z + 1, parent_w)];
                let rare = (noise - 2) % 29 == 0;

                let value = if centre != 0 && noise >= 2 && (noise - 2) % 29 == 1 && centre < 128 {
                    if biome::any(centre + 128).is_some() {
                        centre + 128
                    } else {
                        centre
                    }
                } else if self.seed.next_int(3) != 0 && !rare {
                    centre
                } else {
                    let mut hills = self.hill_variant(centre);
                    if rare && hills != centre {
                        hills = if biome::any(hills + 128).is_some() {
                            hills + 128
                        } else {
                            centre
                        };
                    }
                    if hills == centre {
                        centre
                    } else {
                        let neighbours = [
                            parent[index(x + 1, z, parent_w)],
                            parent[index(x + 2, z + 1, parent_w)],
                            parent[index(x, z + 1, parent_w)],
                            parent[index(x + 1, z + 2, parent_w)],
                        ];
                        let matching = neighbours
                            .iter()
                            .filter(|value| biome::equal_or_mesa_plateau(**value, centre))
                            .count();
                        if matching >= 3 {
                            hills
                        } else {
                            centre
                        }
                    }
                };
                out[index(x, z, width)] = value;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        // `GenLayerHills` keeps its variance layer in a private field, not in
        // `parent`, so vanilla's `initWorldGenSeed` never reaches it: the two
        // zoom layers above the hill noise keep `worldGenSeed == 0` for the
        // whole world. Seeding them here silently rewrites the 1:64 Hills map.
        // Pinned by `debug_stage_trace`.
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerShore`: inserts beaches where land meets water.
pub struct GenLayerShore {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerShore {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }

    fn neighbours(parent: &[i32], parent_w: usize, x: usize, z: usize) -> [i32; 4] {
        [
            parent[index(x + 1, z, parent_w)],
            parent[index(x + 2, z + 1, parent_w)],
            parent[index(x, z + 1, parent_w)],
            parent[index(x + 1, z + 2, parent_w)],
        ]
    }

    fn edge_with(
        parent: &[i32],
        parent_w: usize,
        x: usize,
        z: usize,
        centre: i32,
        replacement: i32,
    ) -> i32 {
        if biome::is_oceanic(centre) {
            return centre;
        }
        let neighbours = Self::neighbours(parent, parent_w, x, z);
        if neighbours.iter().any(|value| biome::is_oceanic(*value)) {
            replacement
        } else {
            centre
        }
    }
}

impl Layer for GenLayerShore {
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
                let centre = parent[index(x + 1, z + 1, parent_w)];
                let neighbours = Self::neighbours(&parent, parent_w, x, z);

                let value = if centre == biome_id::MUSHROOM_ISLAND {
                    if neighbours.contains(&biome_id::OCEAN) {
                        biome_id::MUSHROOM_ISLAND_SHORE
                    } else {
                        centre
                    }
                } else if biome::is_jungle_class(centre) {
                    if neighbours.iter().all(|value| biome::shore_accepts(*value)) {
                        if neighbours.iter().any(|value| biome::is_oceanic(*value)) {
                            biome_id::BEACH
                        } else {
                            centre
                        }
                    } else {
                        23
                    }
                } else if matches!(centre, 3 | 34 | 20) {
                    Self::edge_with(&parent, parent_w, x, z, centre, 25)
                } else if biome::is_snowy(centre) {
                    Self::edge_with(&parent, parent_w, x, z, centre, 26)
                } else if matches!(centre, 37 | 38) {
                    let touches_ocean = neighbours.iter().any(|value| biome::is_oceanic(*value));
                    let all_mesa = neighbours.iter().all(|value| biome::is_mesa(*value));
                    if touches_ocean || all_mesa {
                        centre
                    } else {
                        biome_id::DESERT
                    }
                } else if !matches!(
                    centre,
                    biome_id::OCEAN | biome_id::DEEP_OCEAN | biome_id::RIVER | biome_id::SWAMPLAND
                ) {
                    if neighbours.iter().any(|value| biome::is_oceanic(*value)) {
                        biome_id::BEACH
                    } else {
                        centre
                    }
                } else {
                    centre
                };
                out[index(x, z, width)] = value;
            }
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// `GenLayerVoronoiZoom`: the final 1:4 jittered zoom onto block resolution.
pub struct GenLayerVoronoiZoom {
    seed: LayerSeed,
    parent: Box<dyn Layer>,
}

impl GenLayerVoronoiZoom {
    pub fn new(base_seed: i64, parent: Box<dyn Layer>) -> Self {
        Self {
            seed: LayerSeed::new(base_seed),
            parent,
        }
    }
}

impl Layer for GenLayerVoronoiZoom {
    fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
        let area_x = area_x - 2;
        let area_z = area_z - 2;
        let cell_x = area_x >> 2;
        let cell_z = area_z >> 2;
        let parent_w = (width >> 2) + 2;
        let parent_h = (height >> 2) + 2;
        let parent = self.parent.get_ints(cell_x, cell_z, parent_w, parent_h);

        let zoomed_w = (parent_w - 1) << 2;
        let zoomed_h = (parent_h - 1) << 2;
        let mut zoomed = vec![0; zoomed_w * zoomed_h];

        for z in 0..parent_h - 1 {
            let mut top_left = parent[index(0, z, parent_w)];
            let mut bottom_left = parent[index(0, z + 1, parent_w)];
            for x in 0..parent_w - 1 {
                self.seed.init_chunk(
                    ((x as i32 + cell_x) << 2) as i64,
                    ((z as i32 + cell_z) << 2) as i64,
                );
                let top_left_x = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6;
                let top_left_z = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6;

                self.seed.init_chunk(
                    ((x as i32 + cell_x + 1) << 2) as i64,
                    ((z as i32 + cell_z) << 2) as i64,
                );
                let top_right_x = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6 + 4.0;
                let top_right_z = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6;

                self.seed.init_chunk(
                    ((x as i32 + cell_x) << 2) as i64,
                    ((z as i32 + cell_z + 1) << 2) as i64,
                );
                let bottom_left_x = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6;
                let bottom_left_z = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6 + 4.0;

                self.seed.init_chunk(
                    ((x as i32 + cell_x + 1) << 2) as i64,
                    ((z as i32 + cell_z + 1) << 2) as i64,
                );
                let bottom_right_x = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6 + 4.0;
                let bottom_right_z = (self.seed.next_int(1024) as f64 / 1024.0 - 0.5) * 3.6 + 4.0;

                let top_right_biome = parent[index(x + 1, z, parent_w)] & 255;
                let bottom_right_biome = parent[index(x + 1, z + 1, parent_w)] & 255;

                for dz in 0..4usize {
                    let row = ((z << 2) + dz) * zoomed_w + (x << 2);
                    for dx in 0..4usize {
                        let dxf = dx as f64;
                        let dzf = dz as f64;
                        // Vanilla pairs the first draw with the column and the
                        // second with the row.
                        let d0 = (dzf - top_left_z).powi(2) + (dxf - top_left_x).powi(2);
                        let d1 = (dzf - top_right_z).powi(2) + (dxf - top_right_x).powi(2);
                        let d2 = (dzf - bottom_left_z).powi(2) + (dxf - bottom_left_x).powi(2);
                        let d3 = (dzf - bottom_right_z).powi(2) + (dxf - bottom_right_x).powi(2);
                        zoomed[row + dx] = if d0 < d1 && d0 < d2 && d0 < d3 {
                            top_left
                        } else if d1 < d0 && d1 < d2 && d1 < d3 {
                            top_right_biome
                        } else if d2 < d0 && d2 < d1 && d2 < d3 {
                            bottom_left
                        } else {
                            bottom_right_biome
                        };
                    }
                }
                top_left = top_right_biome;
                bottom_left = bottom_right_biome;
            }
        }

        let mut out = vec![0; width * height];
        let x_offset = (area_x & 3) as usize;
        let z_offset = (area_z & 3) as usize;
        for z in 0..height {
            let src = (z + z_offset) * zoomed_w + x_offset;
            let len = width.min(zoomed_w.saturating_sub(x_offset));
            out[index(0, z, width)..index(0, z, width) + len]
                .copy_from_slice(&zoomed[src..src + len]);
        }
        out
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.parent.set_world_seed(seed);
        self.seed.set_world_seed(seed);
    }
}

/// The assembled biome stack: `GenLayer.initializeAllBiomeGenerators` for a
/// default world.
pub struct BiomeLayers {
    voronoi: Box<dyn Layer>,
    debug_mixed: SharedLayer,
    debug_hills_noise: SharedLayer,
    debug_base: SharedLayer,
    debug_chain: SharedLayer,
    debug_smoothed: SharedLayer,
    debug_biome_edge: SharedLayer,
    debug_hills_only: SharedLayer,
}

impl BiomeLayers {
    pub fn new(seed: i64) -> Self {
        let mut layer: Box<dyn Layer> = Box::new(GenLayerIsland::new(1));
        layer = Box::new(GenLayerZoom::fuzzy(2000, layer));
        layer = Box::new(GenLayerAddIsland::new(1, layer));
        layer = Box::new(GenLayerZoom::new(2001, layer));
        layer = Box::new(GenLayerAddIsland::new(2, layer));
        layer = Box::new(GenLayerAddIsland::new(50, layer));
        layer = Box::new(GenLayerAddIsland::new(70, layer));
        layer = Box::new(GenLayerRemoveTooMuchOcean::new(2, layer));
        layer = Box::new(GenLayerAddSnow::new(2, layer));
        layer = Box::new(GenLayerAddIsland::new(3, layer));
        layer = Box::new(GenLayerEdge::new(2, layer, EdgeMode::CoolWarm));
        layer = Box::new(GenLayerEdge::new(2, layer, EdgeMode::HeatIce));
        layer = Box::new(GenLayerEdge::new(3, layer, EdgeMode::Special));
        layer = Box::new(GenLayerZoom::new(2002, layer));
        layer = Box::new(GenLayerZoom::new(2003, layer));
        layer = Box::new(GenLayerAddIsland::new(4, layer));
        layer = Box::new(GenLayerAddMushroomIsland::new(5, layer));
        layer = Box::new(GenLayerDeepOcean::new(4, layer));

        // Both branches read the same coarse map, as vanilla shares `genlayer4`.
        let base = SharedLayer::new(layer);
        let river_init = SharedLayer::new(Box::new(GenLayerRiverInit::new(100, base.boxed())));

        let biome_source = Box::new(GenLayerBiome::new(200, base.boxed()));
        let zoomed_biomes = magnify(1000, biome_source, 2);
        let biome_edge = SharedLayer::new(Box::new(GenLayerBiomeEdge::new(1000, zoomed_biomes)));

        let hill_noise = SharedLayer::new(magnify(1000, river_init.boxed(), 2));
        let mut chain: Box<dyn Layer> = Box::new(GenLayerHills::new(
            1000,
            biome_edge.boxed(),
            hill_noise.boxed(),
        ));

        let mut river_chain = magnify(1000, river_init.boxed(), 2);
        river_chain = magnify(1000, river_chain, 4);
        let river = Box::new(GenLayerRiver::new(1, river_chain));
        let smoothed_river = Box::new(GenLayerSmooth::new(1000, river));

        let hills_only = SharedLayer::new(chain);
        chain = Box::new(GenLayerRareBiome::new(1001, hills_only.boxed()));

        for step in 0..4i64 {
            chain = Box::new(GenLayerZoom::new(1000 + step, chain));
            if step == 0 {
                chain = Box::new(GenLayerAddIsland::new(3, chain));
            }
            if step == 1 {
                chain = Box::new(GenLayerShore::new(1000, chain));
            }
        }

        let chain = SharedLayer::new(chain);
        let smoothed = SharedLayer::new(Box::new(GenLayerSmooth::new(1000, chain.boxed())));
        let mixed = SharedLayer::new(Box::new(GenLayerRiverMix::new(
            100,
            smoothed.boxed(),
            smoothed_river,
        )));
        let voronoi = Box::new(GenLayerVoronoiZoom::new(10, mixed.boxed()));

        let mut layers = Self {
            voronoi,
            debug_mixed: mixed,
            debug_hills_noise: hill_noise,
            debug_base: base,
            debug_chain: chain,
            debug_smoothed: smoothed,
            debug_biome_edge: biome_edge,
            debug_hills_only: hills_only,
        };
        layers.set_world_seed(seed);
        layers
    }

    fn set_world_seed(&mut self, seed: i64) {
        self.voronoi.set_world_seed(seed);
    }

    pub fn debug_base(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_base.get_ints(x, z, w, h)
    }

    pub fn debug_mixed(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_mixed.get_ints(x, z, w, h)
    }

    pub fn debug_hills_noise(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_hills_noise.get_ints(x, z, w, h)
    }

    /// The chain just before `GenLayerSmooth`.
    pub fn debug_chain(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_chain.get_ints(x, z, w, h)
    }

    /// The chain after `GenLayerSmooth`, before rivers are mixed in.
    pub fn debug_smoothed(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_smoothed.get_ints(x, z, w, h)
    }

    /// `GenLayerBiomeEdge`, the last layer before `GenLayerHills`.
    pub fn debug_biome_edge(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_biome_edge.get_ints(x, z, w, h)
    }

    /// `GenLayerHills` output, before `GenLayerRareBiome`.
    pub fn debug_hills_only(&mut self, x: i32, z: i32, w: usize, h: usize) -> Vec<i32> {
        self.debug_hills_only.get_ints(x, z, w, h)
    }

    /// Biome ids for a block-space rectangle, one per block.
    pub fn biomes(&mut self, block_x: i32, block_z: i32, width: usize, height: usize) -> Vec<i32> {
        self.voronoi.get_ints(block_x, block_z, width, height)
    }

    /// The 16x16 biome map for one chunk, X fastest, as `Level.Biomes` stores it.
    pub fn chunk_biomes(&mut self, chunk_x: i32, chunk_z: i32) -> Vec<i32> {
        self.biomes(chunk_x * 16, chunk_z * 16, 16, 16)
    }

    /// The biome sample the terrain height field is built from.
    ///
    /// `GenLayer.initializeAllBiomeGenerators` returns `{rivermix, voronoi,
    /// rivermix}`, and `WorldChunkManager.getBiomesForGeneration` reads the
    /// first one: the coarse 1:64 river-mixed layer. Only the stored per-block
    /// map comes from the voronoi layer, so terrain must not use [`biomes`].
    pub fn terrain_biomes(&mut self, x: i32, z: i32, width: usize, height: usize) -> Vec<i32> {
        self.debug_mixed.get_ints(x, z, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

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

    /// `GenLayerHills` holds its variance layer outside `parent`, so vanilla's
    /// `initWorldGenSeed` never reaches it and the two zooms above the hill
    /// noise keep `worldGenSeed == 0`. That quirk decides 1 in 29 of the 1:64
    /// cells, so seeding it "correctly" rewrites the whole biome map.
    #[test]
    fn hills_never_seeds_its_variance_layer() {
        use std::cell::Cell;

        struct SeedSpy(Rc<Cell<usize>>);

        impl Layer for SeedSpy {
            fn get_ints(&mut self, _x: i32, _z: i32, width: usize, height: usize) -> Vec<i32> {
                vec![2; width * height]
            }
            fn set_world_seed(&mut self, _seed: i64) {
                self.0.set(self.0.get() + 1);
            }
        }

        let calls = Rc::new(Cell::new(0));
        let mut hills = GenLayerHills::new(
            1000,
            Box::new(Constant(1)),
            Box::new(SeedSpy(Rc::clone(&calls))),
        );
        hills.set_world_seed(12345);
        assert_eq!(
            calls.get(),
            0,
            "vanilla's GenLayerHills never reseeds field_151628_d"
        );
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

    #[test]
    fn the_full_stack_is_deterministic() {
        let mut a = BiomeLayers::new(12345);
        let mut b = BiomeLayers::new(12345);
        assert_eq!(
            a.chunk_biomes(-28, 4),
            b.chunk_biomes(-28, 4),
            "same seed, same world"
        );

        let mut c = BiomeLayers::new(54321);
        assert_ne!(
            a.chunk_biomes(-28, 4),
            c.chunk_biomes(-28, 4),
            "different seed, different world"
        );
    }

    /// The mutation ids vanilla actually registers in `BiomeGenBase`'s static
    /// block. A mutation is only usable when its base id appears here.
    const REGISTERED_MUTATION_BASES: [i32; 21] = [
        1, 2, 3, 4, 5, 6, 12, 21, 23, 27, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39,
    ];

    struct ReferenceCell {
        counts: [u32; 256],
        covered: u32,
    }

    impl Default for ReferenceCell {
        fn default() -> Self {
            Self {
                counts: [0; 256],
                covered: 0,
            }
        }
    }

    impl ReferenceCell {
        fn add(&mut self, biome: u8) {
            self.counts[biome as usize] += 1;
            self.covered += 1;
        }

        fn majority(&self) -> Option<(i32, u32)> {
            let mut best = None;
            for (id, count) in self.counts.iter().copied().enumerate() {
                if count == 0 {
                    continue;
                }
                if best.is_none_or(|(_, best_count)| count > best_count) {
                    best = Some((id as i32, count));
                }
            }
            best
        }
    }

    fn base_id(id: i32) -> i32 {
        if id >= biome::MUTATED {
            id - biome::MUTATED
        } else {
            id
        }
    }

    fn is_registered_mutation(id: i32) -> bool {
        id >= biome::MUTATED && REGISTERED_MUTATION_BASES.contains(&base_id(id))
    }

    #[ignore = "diagnostic: measures where a 1:64 Hills cell lands in block space"]
    #[test]
    fn debug_hills_to_block_mapping() {
        struct Spike {
            x: i32,
            z: i32,
        }

        impl Layer for Spike {
            fn get_ints(
                &mut self,
                area_x: i32,
                area_z: i32,
                width: usize,
                height: usize,
            ) -> Vec<i32> {
                let mut out = vec![5; width * height];
                for z in 0..height as i32 {
                    for x in 0..width as i32 {
                        if area_x + x == self.x && area_z + z == self.z {
                            out[(z * width as i32 + x) as usize] = 133;
                        }
                    }
                }
                out
            }

            fn set_world_seed(&mut self, _seed: i64) {}
        }

        let mut layer: Box<dyn Layer> = Box::new(Spike { x: -2, z: 1 });
        layer = Box::new(GenLayerRareBiome::new(1001, layer));
        for step in 0..4i64 {
            layer = Box::new(GenLayerZoom::new(1000 + step, layer));
            if step == 0 {
                layer = Box::new(GenLayerAddIsland::new(3, layer));
            }
            if step == 1 {
                layer = Box::new(GenLayerShore::new(1000, layer));
            }
        }
        let smoothed = Box::new(GenLayerSmooth::new(1000, layer));
        let mixed = Box::new(GenLayerRiverMix::new(100, smoothed, Box::new(Constant(0))));
        let mut voronoi = GenLayerVoronoiZoom::new(10, mixed);

        let (x, z, w, h) = (-256, -256, 512, 1024);
        let map = voronoi.get_ints(x, z, w as usize, h as usize);
        let mut min = (i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN);
        let mut count = 0usize;
        for bz in 0..h {
            for bx in 0..w {
                if map[(bz * w + bx) as usize] == 133 {
                    min.0 = min.0.min(bx + x);
                    min.1 = min.1.min(bz + z);
                    max.0 = max.0.max(bx + x);
                    max.1 = max.1.max(bz + z);
                    count += 1;
                }
            }
        }
        println!("spike (-2,1) -> blocks {min:?}..{max:?}, {count} cells");
    }

    #[ignore = "diagnostic: checks the Hills layer's query-window dependence"]
    #[test]
    fn debug_hills_query_window() {
        let targets = [(-2, 1), (-3, -1), (-3, 0)];
        let windows = [
            (-8, -4, 8usize, 16usize),
            (-16, -16, 32, 32),
            (-256, -256, 512, 512),
            (-256, -256, 512, 1024),
            (-512, -512, 1024, 1024),
        ];
        for (x, z, w, h) in windows {
            let mut layers = BiomeLayers::new(12345);
            let hills = layers.debug_hills_only(x, z, w, h);
            let noises = layers.debug_hills_noise(x, z, w, h);
            for target in targets {
                let index = ((target.1 - z) * w as i32 + (target.0 - x)) as usize;
                println!(
                    "window ({x},{z},{w},{h}) at {target:?}: hills {:?} noise {:?}",
                    hills.get(index).copied(),
                    noises.get(index).copied()
                );
            }
        }
    }

    #[test]
    fn biome_results_do_not_depend_on_query_rectangle_size() {
        let (area_x, area_z) = (-160, -64);
        let (width, height) = (16usize, 16usize);
        let mut rectangle = BiomeLayers::new(12345);
        let expected = rectangle.biomes(area_x, area_z, width, height);

        let mut points = BiomeLayers::new(12345);
        let mut actual = Vec::with_capacity(width * height);
        for z in 0..height {
            for x in 0..width {
                actual.extend(points.biomes(area_x + x as i32, area_z + z as i32, 1, 1));
            }
        }
        assert_eq!(actual, expected);
    }

    /// Compare the 1:64 `GenLayerHills` grid with a downsampled oracle: for
    /// every 64x64 block square in the reference world, take the majority
    /// biome and compare it with the Hills cell that covers that square.
    ///
    /// Only fully covered squares are scored, so a partially generated square
    /// cannot bias its majority toward whichever chunks happened to exist.
    #[ignore = "diagnostic: compares the 1:64 Hills grid with the reference world"]
    #[test]
    fn debug_hills_downsampled_confusion() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        let mut cells: BTreeMap<(i32, i32), ReferenceCell> = BTreeMap::new();
        let mut chunks = 0usize;

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
                    if stored.biomes.len() != 256 {
                        continue;
                    }
                    chunks += 1;
                    for z in 0..16i32 {
                        for x in 0..16i32 {
                            let block_x = stored.x * 16 + x;
                            let block_z = stored.z * 16 + z;
                            let biome = stored.biomes[(z * 16 + x) as usize];
                            let key = (block_x.div_euclid(64), block_z.div_euclid(64));
                            cells.entry(key).or_default().add(biome);
                        }
                    }
                }
            }
        }

        let full_cells: Vec<_> = cells
            .iter()
            .filter(|(_, cell)| cell.covered == 64 * 64)
            .collect();
        let partial_cells = cells.len() - full_cells.len();
        println!(
            "reference: {chunks} chunks, {} cells, {} fully covered, {} partial",
            cells.len(),
            full_cells.len(),
            partial_cells
        );

        let min_x = full_cells.iter().map(|entry| entry.0 .0).min().unwrap_or(0);
        let min_z = full_cells.iter().map(|entry| entry.0 .1).min().unwrap_or(0);
        let width = full_cells
            .iter()
            .map(|entry| entry.0 .0)
            .max()
            .unwrap_or(min_x)
            - min_x
            + 1;
        let height = full_cells
            .iter()
            .map(|entry| entry.0 .1)
            .max()
            .unwrap_or(min_z)
            - min_z
            + 1;
        let mut layers = BiomeLayers::new(12345);
        // Query the Hills layer as one rectangle. A 1x1 query goes through the
        // zoom layers with a different parent window, so it must not be used to
        // judge the layer's bulk output.
        let hills = layers.debug_hills_only(min_x, min_z, width as usize, height as usize);
        let edges = layers.debug_biome_edge(min_x, min_z, width as usize, height as usize);
        let noises = layers.debug_hills_noise(min_x, min_z, width as usize, height as usize);

        let final_map = layers.biomes(
            min_x * 64,
            min_z * 64,
            (width * 64) as usize,
            (height * 64) as usize,
        );
        let mut final_exact = 0u32;
        let mut final_base = 0u32;
        let mut final_mismatches = Vec::new();
        for (&(cell_x, cell_z), cell) in &full_cells {
            let Some((expected, _)) = cell.majority() else {
                continue;
            };
            let mut actual_cell = ReferenceCell::default();
            for lz in 0..64 {
                for lx in 0..64 {
                    let block_x = cell_x * 64 + lx;
                    let block_z = cell_z * 64 + lz;
                    let index =
                        ((block_z - min_z * 64) * width * 64 + (block_x - min_x * 64)) as usize;
                    actual_cell.add(final_map[index] as u8);
                }
            }
            let Some((actual, _)) = actual_cell.majority() else {
                continue;
            };
            if actual == expected {
                final_exact += 1;
            }
            if base_id(actual) == base_id(expected) {
                final_base += 1;
            } else {
                final_mismatches.push((cell_x, cell_z, expected, actual));
            }
        }
        println!(
            "our final 1:1 map vs reference majority: exact {final_exact}/{}, \
             base {final_base}/{}, cell mismatches {final_mismatches:?}",
            full_cells.len(),
            full_cells.len()
        );
        let mut raw_pairs: BTreeMap<(i32, i32), u32> = BTreeMap::new();
        let mut base_pairs: BTreeMap<(i32, i32), u32> = BTreeMap::new();
        let mut exact = 0u32;
        let mut mutation_only = 0u32;
        let mut different_base = 0u32;
        let mut invalid_actual_mutations = 0u32;

        for (&(cell_x, cell_z), cell) in &full_cells {
            let Some((expected, _)) = cell.majority() else {
                continue;
            };
            let cell_index = ((cell_z - min_z) * width + (cell_x - min_x)) as usize;
            let actual = hills[cell_index];
            let expected_base = base_id(expected);
            let actual_base = base_id(actual);
            if expected != actual {
                let edge_at = |x: i32, z: i32| {
                    if x < min_x || z < min_z {
                        return 0;
                    }
                    let index = ((z - min_z) * width + (x - min_x)) as usize;
                    edges.get(index).copied().unwrap_or(0)
                };
                let neighbours = [
                    edge_at(cell_x, cell_z - 1),
                    edge_at(cell_x + 1, cell_z),
                    edge_at(cell_x - 1, cell_z),
                    edge_at(cell_x, cell_z + 1),
                ];
                let matching = neighbours
                    .iter()
                    .filter(|value| biome::equal_or_mesa_plateau(**value, edges[cell_index]))
                    .count();
                let mut hills_seed = LayerSeed::new(1000);
                hills_seed.set_world_seed(12345);
                hills_seed.init_chunk(cell_x as i64, cell_z as i64);
                let roll = hills_seed.next_int(3);
                println!(
                    "  cell ({cell_x:>3},{cell_z:>3}) expected {expected:>3} \
                     ({}/4096) actual {actual:>3} edge {:>3} neighbours {neighbours:?} \
                     matching {matching} roll {roll} noise {}",
                    cell.majority().map_or(0, |(_, count)| count),
                    edges[cell_index],
                    noises[cell_index]
                );
            }

            *raw_pairs.entry((expected, actual)).or_default() += 1;
            *base_pairs.entry((expected_base, actual_base)).or_default() += 1;

            if expected == actual {
                exact += 1;
            } else if expected_base == actual_base {
                mutation_only += 1;
            } else {
                different_base += 1;
            }
            if actual >= biome::MUTATED && !is_registered_mutation(actual) {
                invalid_actual_mutations += 1;
            }
        }

        let scored = full_cells.len() as u32;
        println!(
            "raw Hills exact: {exact}/{scored} ({:.4}%), mutation-only {mutation_only}, \
             different-base {different_base}, invalid actual mutations {invalid_actual_mutations}",
            if scored == 0 {
                0.0
            } else {
                100.0 * f64::from(exact) / f64::from(scored)
            }
        );

        let mut raw: Vec<_> = raw_pairs.into_iter().collect();
        raw.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        println!("raw mismatches (expected -> actual):");
        for ((expected, actual), count) in raw
            .iter()
            .filter(|((expected, actual), _)| expected != actual)
        {
            println!(
                "  {expected:>3} ({}) -> {actual:>3} ({}): {count}",
                biome::name_of(*expected),
                biome::name_of(*actual)
            );
        }

        let expected_ids: BTreeSet<i32> = base_pairs.keys().map(|(id, _)| *id).collect();
        let actual_ids: BTreeSet<i32> = base_pairs.keys().map(|(_, id)| *id).collect();
        println!("base-id confusion matrix (expected rows, actual columns):");
        print!("      ");
        for id in &actual_ids {
            print!("{id:>5}");
        }
        println!();
        for expected in &expected_ids {
            print!("{expected:>4}: ");
            for actual in &actual_ids {
                let count = base_pairs.get(&(*expected, *actual)).copied().unwrap_or(0);
                if count == 0 {
                    print!("{:>5}", ".");
                } else {
                    print!("{count:>5}");
                }
            }
            println!();
        }
    }

    /// Every chunk the real 1.8.9 server generated must carry the same biome
    /// ids we produce. This is what proves the chain is a faithful port.
    ///
    /// Needs a world at `Nephele-private/parity` (see the oracle's README); it
    /// skips when that world is not present.
    #[ignore = "diagnostic: scans the whole reference region"]
    #[test]
    fn debug_first_mismatch() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        let mut layers = BiomeLayers::new(12345);
        let mut total = 0usize;
        let mut bad = 0usize;
        let mut first = 0usize;
        let mut pairs: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        let offsets = [
            (0, 0),
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ];
        let mut offset_mismatches = [0usize; 9];
        for (name, _base_x, _base_z) in [("r.-1.-1.mca", -32, -32), ("r.-1.0.mca", -32, 0)] {
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
                    let maps: Vec<_> = offsets
                        .iter()
                        .map(|(dx, dz)| {
                            layers.biomes(stored.x * 16 + dx, stored.z * 16 + dz, 16, 16)
                        })
                        .collect();
                    if maps[0].len() != stored.biomes.len() {
                        continue;
                    }
                    for z in 0..16i32 {
                        for x in 0..16i32 {
                            let i = (z * 16 + x) as usize;
                            total += 1;
                            let expected = stored.biomes[i] as i32;
                            for (offset, map) in maps.iter().enumerate() {
                                if map[i] != expected {
                                    offset_mismatches[offset] += 1;
                                }
                            }
                            if maps[0][i] != expected {
                                bad += 1;
                                *pairs.entry((maps[0][i], expected)).or_default() += 1;
                                if first == 0 {
                                    first = 1;
                                    let block_x = stored.x * 16 + x;
                                    let block_z = stored.z * 16 + z;
                                    let parent_x = (block_x - 2) >> 2;
                                    let parent_z = (block_z - 2) >> 2;
                                    let hills_x = parent_x >> 4;
                                    let hills_z = parent_z >> 4;
                                    println!(
                                        "first mismatch: block=({},{}) chunk=({},{}) ours={} real={}",
                                        block_x,
                                        block_z,
                                        stored.x,
                                        stored.z,
                                        maps[0][i],
                                        stored.biomes[i]
                                    );
                                    println!(
                                        "1:4 parents at ({parent_x},{parent_z}): chain={:?} smooth={:?} mixed={:?}",
                                        layers.debug_chain(parent_x - 1, parent_z - 1, 4, 4),
                                        layers.debug_smoothed(parent_x - 1, parent_z - 1, 4, 4),
                                        layers.debug_mixed(parent_x - 1, parent_z - 1, 4, 4),
                                    );
                                    println!(
                                        "hills near ({hills_x},{hills_z})={:?}",
                                        layers.debug_hills_only(hills_x - 2, hills_z - 2, 6, 6),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("mismatched {bad} of {total}");
        let mut pairs: Vec<_> = pairs.into_iter().collect();
        pairs.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
        println!("final block mismatches (ours -> reference), top 30:");
        for ((ours, reference), count) in pairs.iter().take(30) {
            println!(
                "  {ours:>3} ({}) -> {reference:>3} ({}): {count}",
                biome::name_of(*ours),
                biome::name_of(*reference)
            );
        }
        for (offset, mismatches) in offsets.iter().zip(offset_mismatches) {
            println!("offset {offset:?}: mismatched {mismatches} of {total}");
        }
    }

    /// One `getInts` call, captured by [`Rec`].
    struct Snapshot {
        name: String,
        x: i32,
        z: i32,
        w: usize,
        h: usize,
        data: Vec<i32>,
    }

    /// Records every layer's output on the way back up the chain. Mirrors
    /// `dev.nephele.oracle.Trace`'s `Rec` so the two traces line up call for
    /// call: the oracle snapshots each layer inside one continuous generation,
    /// which avoids the `IntCache` aliasing that makes isolated stage queries
    /// contradict each other.
    struct Rec {
        name: &'static str,
        inner: Box<dyn Layer>,
        log: Rc<RefCell<Vec<Snapshot>>>,
    }

    impl Layer for Rec {
        fn get_ints(&mut self, area_x: i32, area_z: i32, width: usize, height: usize) -> Vec<i32> {
            let out = self.inner.get_ints(area_x, area_z, width, height);
            self.log.borrow_mut().push(Snapshot {
                name: self.name.to_string(),
                x: area_x,
                z: area_z,
                w: width,
                h: height,
                data: out.clone(),
            });
            out
        }

        fn set_world_seed(&mut self, seed: i64) {
            self.inner.set_world_seed(seed);
        }
    }

    fn rec(
        name: &'static str,
        inner: Box<dyn Layer>,
        log: &Rc<RefCell<Vec<Snapshot>>>,
    ) -> Box<dyn Layer> {
        Box::new(Rec {
            name,
            inner,
            log: Rc::clone(log),
        })
    }

    /// The oracle's stack, rebuilt node for node with the same names (see
    /// `Nephele-private/oracle/src/dev/nephele/oracle/Trace.java`).
    fn traced_stack(log: &Rc<RefCell<Vec<Snapshot>>>) -> Box<dyn Layer> {
        let island = rec("island", Box::new(GenLayerIsland::new(1)), log);
        let fuzzy = rec(
            "fuzzyZoom",
            Box::new(GenLayerZoom::fuzzy(2000, island)),
            log,
        );
        let add1 = rec("add1", Box::new(GenLayerAddIsland::new(1, fuzzy)), log);
        let zoom2001 = rec("zoom2001", Box::new(GenLayerZoom::new(2001, add1)), log);
        let add2 = rec("add2", Box::new(GenLayerAddIsland::new(2, zoom2001)), log);
        let add3 = rec("add3", Box::new(GenLayerAddIsland::new(50, add2)), log);
        let add4 = rec("add4", Box::new(GenLayerAddIsland::new(70, add3)), log);
        let ocean = rec(
            "removeOcean",
            Box::new(GenLayerRemoveTooMuchOcean::new(2, add4)),
            log,
        );
        let snow = rec("addSnow", Box::new(GenLayerAddSnow::new(2, ocean)), log);
        let add5 = rec("add5", Box::new(GenLayerAddIsland::new(3, snow)), log);
        let edge1 = rec(
            "edgeCoolWarm",
            Box::new(GenLayerEdge::new(2, add5, EdgeMode::CoolWarm)),
            log,
        );
        let edge2 = rec(
            "edgeHeatIce",
            Box::new(GenLayerEdge::new(2, edge1, EdgeMode::HeatIce)),
            log,
        );
        let edge3 = rec(
            "edgeSpecial",
            Box::new(GenLayerEdge::new(3, edge2, EdgeMode::Special)),
            log,
        );
        let zoom2002 = rec("zoom2002", Box::new(GenLayerZoom::new(2002, edge3)), log);
        let zoom2003 = rec("zoom2003", Box::new(GenLayerZoom::new(2003, zoom2002)), log);
        let add6 = rec("add6", Box::new(GenLayerAddIsland::new(4, zoom2003)), log);
        let mushroom = rec(
            "mushroom",
            Box::new(GenLayerAddMushroomIsland::new(5, add6)),
            log,
        );
        let deep = rec(
            "deepOcean",
            Box::new(GenLayerDeepOcean::new(4, mushroom)),
            log,
        );

        let base = SharedLayer::new(deep);
        let river_init = SharedLayer::new(rec(
            "riverInit",
            Box::new(GenLayerRiverInit::new(100, base.boxed())),
            log,
        ));

        let biome_source = rec(
            "biomeSource",
            Box::new(GenLayerBiome::new(200, base.boxed())),
            log,
        );
        let bio_zoom0 = rec(
            "bioZoom0",
            Box::new(GenLayerZoom::new(1000, biome_source)),
            log,
        );
        let bio_zoom1 = rec(
            "bioZoom1",
            Box::new(GenLayerZoom::new(1001, bio_zoom0)),
            log,
        );
        let biome_edge = rec(
            "biomeEdge",
            Box::new(GenLayerBiomeEdge::new(1000, bio_zoom1)),
            log,
        );

        let hill_noise0 = rec(
            "hillNoiseZoom0",
            Box::new(GenLayerZoom::new(1000, river_init.boxed())),
            log,
        );
        let hill_noise1 = rec(
            "hillNoiseZoom1",
            Box::new(GenLayerZoom::new(1001, hill_noise0)),
            log,
        );
        let hills = rec(
            "hills",
            Box::new(GenLayerHills::new(1000, biome_edge, hill_noise1)),
            log,
        );

        let river_zoom0 = rec(
            "riverZoom0",
            Box::new(GenLayerZoom::new(1000, river_init.boxed())),
            log,
        );
        let river_zoom1 = rec(
            "riverZoom1",
            Box::new(GenLayerZoom::new(1001, river_zoom0)),
            log,
        );
        let river_zoom2 = rec(
            "riverZoom2",
            Box::new(GenLayerZoom::new(1000, river_zoom1)),
            log,
        );
        let river_zoom3 = rec(
            "riverZoom3",
            Box::new(GenLayerZoom::new(1001, river_zoom2)),
            log,
        );
        let river_zoom4 = rec(
            "riverZoom4",
            Box::new(GenLayerZoom::new(1002, river_zoom3)),
            log,
        );
        let river_zoom5 = rec(
            "riverZoom5",
            Box::new(GenLayerZoom::new(1003, river_zoom4)),
            log,
        );
        let river = rec("river", Box::new(GenLayerRiver::new(1, river_zoom5)), log);
        let river_smooth = rec(
            "riverSmooth",
            Box::new(GenLayerSmooth::new(1000, river)),
            log,
        );

        let rare = rec(
            "rareBiome",
            Box::new(GenLayerRareBiome::new(1001, hills)),
            log,
        );
        let chain_zoom0 = rec("chainZoom0", Box::new(GenLayerZoom::new(1000, rare)), log);
        let chain_add = rec(
            "chainAddIsland",
            Box::new(GenLayerAddIsland::new(3, chain_zoom0)),
            log,
        );
        let chain_zoom1 = rec(
            "chainZoom1",
            Box::new(GenLayerZoom::new(1001, chain_add)),
            log,
        );
        let chain_shore = rec(
            "chainShore",
            Box::new(GenLayerShore::new(1000, chain_zoom1)),
            log,
        );
        let chain_zoom2 = rec(
            "chainZoom2",
            Box::new(GenLayerZoom::new(1002, chain_shore)),
            log,
        );
        let chain_zoom3 = rec(
            "chainZoom3",
            Box::new(GenLayerZoom::new(1003, chain_zoom2)),
            log,
        );
        let smooth = rec(
            "smooth",
            Box::new(GenLayerSmooth::new(1000, chain_zoom3)),
            log,
        );
        let mixed = rec(
            "riverMix",
            Box::new(GenLayerRiverMix::new(100, smooth, river_smooth)),
            log,
        );
        rec(
            "voronoi",
            Box::new(GenLayerVoronoiZoom::new(10, mixed)),
            log,
        )
    }

    fn read_trace(path: &std::path::Path) -> Vec<Snapshot> {
        use std::io::Read;

        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .expect("stage trace exists")
            .read_to_end(&mut bytes)
            .expect("stage trace reads");
        assert_eq!(&bytes[..8], b"NEPHTRC1", "stage trace magic");
        let mut cursor = 8usize;
        let read_i32 = |cursor: &mut usize| {
            let value = i32::from_be_bytes(bytes[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            value
        };
        let count = read_i32(&mut cursor) as usize;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let name_len = read_i32(&mut cursor) as usize;
            let name = String::from_utf8(bytes[cursor..cursor + name_len].to_vec()).unwrap();
            cursor += name_len;
            let x = read_i32(&mut cursor);
            let z = read_i32(&mut cursor);
            let w = read_i32(&mut cursor) as usize;
            let h = read_i32(&mut cursor) as usize;
            let len = read_i32(&mut cursor) as usize;
            let mut data = Vec::with_capacity(len);
            for _ in 0..len {
                data.push(read_i32(&mut cursor));
            }
            out.push(Snapshot {
                name,
                x,
                z,
                w,
                h,
                data,
            });
        }
        out
    }

    /// Reports which stage first diverges from the vanilla trace. Needs
    /// `Nephele-private/parity/stage_trace.bin`, produced by
    /// `dev.nephele.oracle.Trace <seed> -512 -512 512 1024 <out>`.
    #[ignore = "diagnostic: stage-by-stage diff against the vanilla trace"]
    #[test]
    fn debug_stage_trace() {
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/stage_trace.bin");
        if !path.is_file() {
            eprintln!("skipped: no stage trace at {}", path.display());
            return;
        }
        let expected = read_trace(&path);

        let log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = traced_stack(&log);
        stack.set_world_seed(12345);
        stack.get_ints(-512, -512, 512, 1024);
        let actual = log.borrow();

        println!("records: oracle {} ours {}", expected.len(), actual.len());
        let mut first_bad = true;
        for (index, (want, got)) in expected.iter().zip(actual.iter()).enumerate() {
            let same_window =
                want.x == got.x && want.z == got.z && want.w == got.w && want.h == got.h;
            let mut mismatches = 0usize;
            let mut first_cell = None;
            let cells = want.w * want.h;
            if same_window && want.data.len() >= cells && got.data.len() >= cells {
                for cell in 0..cells {
                    if want.data[cell] != got.data[cell] {
                        mismatches += 1;
                        if first_cell.is_none() {
                            first_cell = Some((
                                cell % want.w,
                                cell / want.w,
                                want.data[cell],
                                got.data[cell],
                            ));
                        }
                    }
                }
            } else {
                mismatches = want.data.len().max(got.data.len());
            }
            let mark = if mismatches == 0 { "ok  " } else { "DIFF" };
            println!(
                "{mark} #{index:>2} {:<14} want ({},{},{},{}) got ({},{},{},{}) diff {}/{} {:?}",
                want.name,
                want.x,
                want.z,
                want.w,
                want.h,
                got.x,
                got.z,
                got.w,
                got.h,
                mismatches,
                want.data.len(),
                first_cell
            );
            if mismatches > 0 && first_bad {
                first_bad = false;
                println!("^ first diverging stage");
            }
        }
    }

    #[test]
    fn chunk_biomes_match_a_world_the_real_server_generated() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let region_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region/r.-1.0.mca");
        if !region_path.is_file() {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        }

        let region = RegionFile::open(&region_path).expect("region opens");
        // Chunk (-28, 4) sits in region (-1, 0) at local (4, 4).
        let bytes = region
            .chunk_nbt(4, 4)
            .expect("chunk reads")
            .expect("chunk present");
        let stored = parse_chunk(&bytes).expect("chunk parses");
        assert_eq!(
            (stored.x, stored.z),
            (-28, 4),
            "reference chunk coordinates"
        );

        let mut layers = BiomeLayers::new(12345);
        let ours = layers.chunk_biomes(stored.x, stored.z);
        assert_eq!(ours.len(), stored.biomes.len());

        let mut mismatches = Vec::new();
        for z in 0..16 {
            for x in 0..16 {
                let index = z * 16 + x;
                let expected = stored.biomes[index] as i32;
                if ours[index] != expected {
                    mismatches.push((x, z, ours[index], expected));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} of 256 biomes differ, first few: {:?}",
            mismatches.len(),
            &mismatches[..mismatches.len().min(8)]
        );
    }
}
