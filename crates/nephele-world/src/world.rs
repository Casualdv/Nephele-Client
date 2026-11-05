use std::collections::HashMap;

use crate::block::BlockState;
use crate::chunk::{ChunkColumn, SECTION_DIM};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
}

impl From<(i32, i32)> for ChunkCoord {
    fn from((x, z): (i32, i32)) -> Self {
        Self { x, z }
    }
}

impl From<(f64, f64)> for ChunkCoord {
    fn from((x, z): (f64, f64)) -> Self {
        // Minecraft chunks are 16 blocks wide; floor-divide so negatives work.
        Self {
            x: (x.floor() as i32).div_euclid(SECTION_DIM as i32),
            z: (z.floor() as i32).div_euclid(SECTION_DIM as i32),
        }
    }
}

/// A loaded world region. Dimension 0 (overworld) has skylight; -1 (nether)
/// and 1 (end) do not.
#[derive(Debug, Clone)]
pub struct World {
    chunks: HashMap<ChunkCoord, ChunkColumn>,
    dimension: i8,
    has_skylight: bool,
}

impl World {
    pub fn new(dimension: i8) -> Self {
        Self {
            chunks: HashMap::new(),
            dimension,
            has_skylight: dimension == 0,
        }
    }

    pub fn dimension(&self) -> i8 {
        self.dimension
    }

    pub fn has_skylight(&self) -> bool {
        self.has_skylight
    }

    /// Changing dimension invalidates every loaded chunk.
    pub fn set_dimension(&mut self, dimension: i8) {
        self.dimension = dimension;
        self.has_skylight = dimension == 0;
        self.chunks.clear();
    }

    pub fn insert(&mut self, column: ChunkColumn) -> Option<ChunkColumn> {
        self.chunks
            .insert(ChunkCoord::new(column.x, column.z), column)
    }

    pub fn remove(&mut self, coord: ChunkCoord) -> Option<ChunkColumn> {
        self.chunks.remove(&coord)
    }

    pub fn get(&self, coord: ChunkCoord) -> Option<&ChunkColumn> {
        self.chunks.get(&coord)
    }

    pub fn get_mut(&mut self, coord: ChunkCoord) -> Option<&mut ChunkColumn> {
        self.chunks.get_mut(&coord)
    }

    pub fn contains(&self, coord: ChunkCoord) -> bool {
        self.chunks.contains_key(&coord)
    }

    pub fn loaded_chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ChunkCoord, &ChunkColumn)> {
        self.chunks.iter()
    }

    pub fn total_non_air(&self) -> u64 {
        self.chunks
            .values()
            .map(|column| column.non_air_count() as u64)
            .sum()
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> BlockState {
        if y < 0 || y >= (crate::chunk::SECTION_COUNT * SECTION_DIM) as i32 {
            return BlockState::AIR;
        }
        let coord = ChunkCoord::new(
            x.div_euclid(SECTION_DIM as i32),
            z.div_euclid(SECTION_DIM as i32),
        );
        match self.chunks.get(&coord) {
            Some(column) => column.get_block(
                x.rem_euclid(SECTION_DIM as i32) as usize,
                y as usize,
                z.rem_euclid(SECTION_DIM as i32) as usize,
            ),
            None => BlockState::AIR,
        }
    }

    /// Write a single block. Returns `false` when the containing column (or the
    /// y range) isn't loaded.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) -> bool {
        if y < 0 || y >= (crate::chunk::SECTION_COUNT * SECTION_DIM) as i32 {
            return false;
        }
        let coord = ChunkCoord::new(
            x.div_euclid(SECTION_DIM as i32),
            z.div_euclid(SECTION_DIM as i32),
        );
        let Some(column) = self.chunks.get_mut(&coord) else {
            return false;
        };
        column.set_block(
            x.rem_euclid(SECTION_DIM as i32) as usize,
            y as usize,
            z.rem_euclid(SECTION_DIM as i32) as usize,
            state,
        );
        true
    }

    /// Highest non-air block at world x/z, searching only loaded columns.
    pub fn height_at(&self, x: i32, z: i32) -> Option<usize> {
        let coord = ChunkCoord::new(
            x.div_euclid(SECTION_DIM as i32),
            z.div_euclid(SECTION_DIM as i32),
        );
        self.chunks.get(&coord).and_then(|column| {
            column.height_at(
                x.rem_euclid(SECTION_DIM as i32) as usize,
                z.rem_euclid(SECTION_DIM as i32) as usize,
            )
        })
    }

    /// Count of every non-air block by block id, most common first.
    pub fn block_histogram(&self) -> Vec<(u16, u64)> {
        let mut counts: HashMap<u16, u64> = HashMap::new();
        for column in self.chunks.values() {
            for (_, section) in column.iter_sections() {
                for raw in section.blocks() {
                    if *raw != 0 {
                        *counts.entry(raw >> 4).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut out: Vec<(u16, u64)> = counts.into_iter().collect();
        out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coord_floors_negatives() {
        assert_eq!(ChunkCoord::from((-1.0, -17.0)), ChunkCoord::new(-1, -2));
        assert_eq!(ChunkCoord::from((15.9, 16.0)), ChunkCoord::new(0, 1));
    }

    #[test]
    fn missing_chunks_read_as_air() {
        let world = World::new(0);
        assert!(world.get_block(0, 64, 0).is_air());
        assert_eq!(world.height_at(0, 0), None);
    }

    #[test]
    fn dimension_switch_clears_chunks() {
        let mut world = World::new(0);
        world.insert(ChunkColumn::empty(0, 0));
        assert_eq!(world.loaded_chunks(), 1);
        assert!(world.has_skylight());
        world.set_dimension(-1);
        assert_eq!(world.loaded_chunks(), 0);
        assert!(!world.has_skylight());
    }

    #[test]
    fn world_coordinates_map_into_chunks() {
        let mut world = World::new(0);
        let mut column = ChunkColumn::empty(-1, 0);
        column.set_block(15, 5, 3, BlockState::STONE);
        world.insert(column);
        // chunk -1 covers x in [-16, -1]
        assert_eq!(world.get_block(-1, 5, 3), BlockState::STONE);
        assert!(world.get_block(0, 5, 3).is_air());
    }
}
