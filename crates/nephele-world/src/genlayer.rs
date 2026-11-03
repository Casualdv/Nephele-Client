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
        self.hill_noise.set_world_seed(seed);
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

    /// Every chunk the real 1.8.9 server generated must carry the same biome
    /// ids we produce. This is what proves the chain is a faithful port.
    ///
    /// Needs a world at `Nephele-private/parity` (see the oracle's README); it
    /// skips when that world is not present.
    #[ignore = "diagnostic: scans the whole reference region"]
    #[test]
    fn debug_first_mismatch() {
        use nephele_anvil::{parse_chunk, RegionFile};
        use std::path::PathBuf;

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region");
        let mut layers = BiomeLayers::new(12345);
        let mut total = 0usize;
        let mut bad = 0usize;
        let mut first = 0usize;
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
                    let ours = layers.chunk_biomes(stored.x, stored.z);
                    if ours.len() != stored.biomes.len() {
                        continue;
                    }
                    for z in 0..16i32 {
                        for x in 0..16i32 {
                            let i = (z * 16 + x) as usize;
                            total += 1;
                            if ours[i] != stored.biomes[i] as i32 {
                                bad += 1;
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
                                        ours[i],
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
