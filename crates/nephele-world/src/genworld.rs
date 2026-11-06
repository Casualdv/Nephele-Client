//! The world view vanilla generators are transliterated against.
//!
//! Vanilla's `ChunkProviderGenerate` and every `WorldGen*` feature talk to
//! `World` through a handful of calls: `getBlockState`, `setBlockState`,
//! `getHeight`, `chunkExists`, `Chunk.isTerrainPopulated`, the sea level and the
//! block-level `Material` predicates. Porting a generator against
//! [`crate::World`] directly would drag chunk lifecycle, lighting and entity
//! state into the port and make the transliteration unreadable.
//!
//! So generation code takes a [`GenWorld`] instead — a trait with exactly those
//! calls — and [`TestWorld`] is a bare `HashMap` of chunks behind it. That is
//! the whole shim: no entities, no block *model*, just block states addressed by
//! world coordinate.

use std::collections::HashMap;

use crate::block::{self, BlockState};
use crate::chunk::{ChunkColumn, SECTION_DIM};
use crate::terrain::SEA_LEVEL;
use crate::world::{ChunkCoord, World};

/// The slice of `World` that generation code is allowed to touch.
///
/// Coordinates are world coordinates (block space), never chunk-local: every
/// vanilla generator writes across chunk borders, so the world, not the chunk,
/// is the unit a population pass sees.
pub trait GenWorld {
    /// `World.getBlockState(pos)`. Air outside loaded chunks and above y=255.
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockState;

    /// `World.setBlockState(pos, state, 2)`. Ignored outside loaded chunks.
    fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState);

    /// `ChunkProvider.chunkExists(chunkX, chunkZ)`.
    fn has_chunk(&self, chunk_x: i32, chunk_z: i32) -> bool;

    /// Store a freshly generated chunk: `ChunkProviderServer`'s "load" step.
    fn insert_chunk(&mut self, column: ChunkColumn);

    /// `Chunk.isTerrainPopulated`.
    fn is_terrain_populated(&self, chunk_x: i32, chunk_z: i32) -> bool;

    /// `Chunk.setTerrainPopulated`.
    fn set_terrain_populated(&mut self, chunk_x: i32, chunk_z: i32, populated: bool);

    /// `World.getHeight(x, z)`: one above the highest light-blocking block of
    /// the column, read from the chunk's cached height map.
    fn height(&self, x: i32, z: i32) -> i32;

    /// `World.getSeaLevel()`.
    fn sea_level(&self) -> i32 {
        SEA_LEVEL
    }

    /// `World.isAirBlock(pos)`.
    fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        self.get_block(x, y, z).is_air()
    }

    /// `Block.getMaterial().isLiquid()` — water and lava, flowing or not.
    fn is_liquid(&self, x: i32, y: i32, z: i32) -> bool {
        block::is_liquid(self.get_block(x, y, z))
    }

    /// `Block.isReplaceable(world, pos)` — what a feature may overwrite.
    fn is_replaceable(&self, x: i32, y: i32, z: i32) -> bool {
        block::is_replaceable(self.get_block(x, y, z))
    }
}

/// A `GenWorld` backed by nothing but a map of chunks.
///
/// Used by the generation tests and the parity harness, where there is no
/// client world, no dimension and no lighting: only blocks and the
/// "has this chunk been populated" flags vanilla keeps on `Chunk`.
#[derive(Debug, Clone)]
pub struct TestWorld {
    chunks: HashMap<ChunkCoord, ChunkColumn>,
    sea_level: i32,
}

impl TestWorld {
    pub fn new() -> Self {
        Self::with_sea_level(SEA_LEVEL)
    }

    pub fn with_sea_level(sea_level: i32) -> Self {
        Self {
            chunks: HashMap::new(),
            sea_level,
        }
    }

    pub fn insert(&mut self, column: ChunkColumn) -> Option<ChunkColumn> {
        self.chunks
            .insert(ChunkCoord::new(column.x, column.z), column)
    }

    pub fn chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&ChunkColumn> {
        self.chunks.get(&ChunkCoord::new(chunk_x, chunk_z))
    }

    pub fn chunk_mut(&mut self, chunk_x: i32, chunk_z: i32) -> Option<&mut ChunkColumn> {
        self.chunks.get_mut(&ChunkCoord::new(chunk_x, chunk_z))
    }

    pub fn loaded_chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ChunkCoord, &ChunkColumn)> {
        self.chunks.iter()
    }
}

impl Default for TestWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Chunk key and chunk-local column index for a world coordinate.
#[inline]
fn split(x: i32, z: i32) -> (ChunkCoord, usize, usize) {
    (
        ChunkCoord::new(
            x.div_euclid(SECTION_DIM as i32),
            z.div_euclid(SECTION_DIM as i32),
        ),
        x.rem_euclid(SECTION_DIM as i32) as usize,
        z.rem_euclid(SECTION_DIM as i32) as usize,
    )
}

impl GenWorld for TestWorld {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockState {
        if !(0..crate::chunk::SECTION_COUNT as i32 * SECTION_DIM as i32).contains(&y) {
            return BlockState::AIR;
        }
        let (coord, local_x, local_z) = split(x, z);
        match self.chunks.get(&coord) {
            Some(column) => column.get_block(local_x, y as usize, local_z),
            None => BlockState::AIR,
        }
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) {
        if !(0..crate::chunk::SECTION_COUNT as i32 * SECTION_DIM as i32).contains(&y) {
            return;
        }
        let (coord, local_x, local_z) = split(x, z);
        if let Some(column) = self.chunks.get_mut(&coord) {
            column.set_block(local_x, y as usize, local_z, state);
        }
    }

    fn has_chunk(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.chunks.contains_key(&ChunkCoord::new(chunk_x, chunk_z))
    }

    fn insert_chunk(&mut self, column: ChunkColumn) {
        self.insert(column);
    }

    fn is_terrain_populated(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.chunk(chunk_x, chunk_z)
            .map(ChunkColumn::is_terrain_populated)
            .unwrap_or(false)
    }

    fn set_terrain_populated(&mut self, chunk_x: i32, chunk_z: i32, populated: bool) {
        if let Some(column) = self.chunk_mut(chunk_x, chunk_z) {
            column.set_terrain_populated(populated);
        }
    }

    fn height(&self, x: i32, z: i32) -> i32 {
        let (coord, local_x, local_z) = split(x, z);
        self.chunks
            .get(&coord)
            .map(|column| column.height_map_at(local_x, local_z))
            .unwrap_or(0)
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }
}

impl GenWorld for World {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockState {
        World::get_block(self, x, y, z)
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) {
        World::set_block(self, x, y, z, state);
    }

    fn has_chunk(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.contains(ChunkCoord::new(chunk_x, chunk_z))
    }

    fn insert_chunk(&mut self, column: ChunkColumn) {
        self.insert(column);
    }

    fn is_terrain_populated(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.get(ChunkCoord::new(chunk_x, chunk_z))
            .map(ChunkColumn::is_terrain_populated)
            .unwrap_or(false)
    }

    fn set_terrain_populated(&mut self, chunk_x: i32, chunk_z: i32, populated: bool) {
        if let Some(column) = self.get_mut(ChunkCoord::new(chunk_x, chunk_z)) {
            column.set_terrain_populated(populated);
        }
    }

    fn height(&self, x: i32, z: i32) -> i32 {
        let (coord, local_x, local_z) = split(x, z);
        self.get(coord)
            .map(|column| column.height_map_at(local_x, local_z))
            .unwrap_or(0)
    }

    fn sea_level(&self) -> i32 {
        SEA_LEVEL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::ids;

    fn small_world() -> TestWorld {
        let mut world = TestWorld::new();
        world.insert(ChunkColumn::empty(0, 0));
        world.insert(ChunkColumn::empty(-1, 0));
        world
    }

    #[test]
    fn blocks_are_addressed_across_chunk_borders() {
        let mut world = small_world();
        // x = -1 is the last column of chunk -1; x = 0 is the first of chunk 0.
        world.set_block(-1, 70, 3, BlockState::new(ids::STONE, 0));
        world.set_block(0, 70, 3, BlockState::new(ids::DIRT, 0));
        assert_eq!(world.get_block(-1, 70, 3).id(), ids::STONE);
        assert_eq!(world.get_block(0, 70, 3).id(), ids::DIRT);
        assert_eq!(world.get_block(1, 70, 3), BlockState::AIR);
    }

    #[test]
    fn writes_to_unloaded_chunks_are_dropped() {
        let mut world = small_world();
        world.set_block(200, 70, 200, BlockState::new(ids::STONE, 0));
        assert!(world.get_block(200, 70, 200).is_air());
    }

    #[test]
    fn out_of_range_heights_read_as_air() {
        let mut world = small_world();
        world.set_block(0, 255, 0, BlockState::new(ids::STONE, 0));
        assert!(!world.get_block(0, 255, 0).is_air());
        assert!(world.get_block(0, 256, 0).is_air());
        assert!(world.get_block(0, -1, 0).is_air());
    }

    #[test]
    fn the_material_predicates_match_vanilla() {
        let mut world = small_world();
        let cases = [
            (BlockState::new(ids::WATER, 0), true, false),
            (BlockState::new(ids::LAVA, 0), true, false),
            (BlockState::new(8, 3), true, false),
            (BlockState::AIR, false, true),
            (BlockState::new(31, 1), false, true),
            (BlockState::new(32, 0), false, true),
            (BlockState::new(175, 0), false, true),
            (BlockState::new(106, 0), false, true),
            (BlockState::new(ids::SNOW_LAYER, 0), false, true),
            (BlockState::new(ids::SNOW_LAYER, 1), false, false),
            (BlockState::new(ids::STONE, 0), false, false),
            (BlockState::new(ids::LOG, 0), false, false),
        ];
        for (index, (state, liquid, replaceable)) in cases.iter().enumerate() {
            world.set_block(0, 60 + index as i32, 0, *state);
            assert_eq!(
                world.is_liquid(0, 60 + index as i32, 0),
                *liquid,
                "{state:?}"
            );
            assert_eq!(
                world.is_replaceable(0, 60 + index as i32, 0),
                *replaceable,
                "{state:?}"
            );
        }
    }

    #[test]
    fn the_population_flag_lives_on_the_chunk() {
        let mut world = small_world();
        assert!(!world.is_terrain_populated(0, 0));
        world.set_terrain_populated(0, 0, true);
        assert!(world.is_terrain_populated(0, 0));
        assert!(!world.is_terrain_populated(1, 0));
        assert!(!world.has_chunk(1, 0));
    }
}
