//! One entry point per dimension, so callers do not have to know which
//! `ChunkProvider*` a world uses.
//!
//! The three generators keep their own `Random` state on purpose. The overworld
//! reseeds per owner chunk inside its population pass; the nether runs
//! `provide_chunk` and then `populate` off one shared stream (vanilla quirk:
//! population depends on load order), and the end seeds a `Random` per owner
//! chunk because vanilla's equivalent is unseeded. [`ChunkGenerator`] therefore
//! exposes the same two calls as each generator and forwards them unchanged.
//!
//! **Call order matters.** `provide_chunk` immediately followed by `populate`
//! reproduces the server's `ChunkProviderServer.loadChunk` → `Chunk.populateChunk`
//! sequence; any other interleaving changes the nether's `Random` stream.

use crate::chunk::ChunkColumn;
use crate::dimension::Dimension;
use crate::end::EndGenerator;
use crate::genworld::GenWorld;
use crate::nether::NetherGenerator;
use crate::world::ChunkCoord;
use crate::worldgen::VanillaGenerator;

/// The chunk generator for one dimension.
///
/// One per world, built once; the variants differ in size because the overworld
/// keeps more noise fields, so boxing them would add an indirection to every
/// `provide_chunk` for no measurable gain.
#[allow(clippy::large_enum_variant)]
pub enum ChunkGenerator {
    Overworld(VanillaGenerator),
    Nether(NetherGenerator),
    End(EndGenerator),
}

impl ChunkGenerator {
    pub fn new(dimension: Dimension, seed: i64) -> Self {
        match dimension {
            Dimension::Overworld => ChunkGenerator::Overworld(VanillaGenerator::new(seed)),
            Dimension::Nether => ChunkGenerator::Nether(NetherGenerator::new(seed)),
            Dimension::End => ChunkGenerator::End(EndGenerator::new(seed)),
        }
    }

    pub fn dimension(&self) -> Dimension {
        match self {
            ChunkGenerator::Overworld(_) => Dimension::Overworld,
            ChunkGenerator::Nether(_) => Dimension::Nether,
            ChunkGenerator::End(_) => Dimension::End,
        }
    }

    pub fn provide_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> ChunkColumn {
        match self {
            ChunkGenerator::Overworld(generator) => generator.provide_chunk(chunk_x, chunk_z),
            ChunkGenerator::Nether(generator) => generator.provide_chunk(chunk_x, chunk_z),
            ChunkGenerator::End(generator) => generator.provide_chunk(chunk_x, chunk_z),
        }
    }

    pub fn populate<W: GenWorld>(
        &mut self,
        world: &mut W,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Vec<ChunkCoord> {
        match self {
            ChunkGenerator::Overworld(generator) => generator.populate(world, chunk_x, chunk_z),
            ChunkGenerator::Nether(generator) => generator.populate(world, chunk_x, chunk_z),
            ChunkGenerator::End(generator) => generator.populate(world, chunk_x, chunk_z),
        }
    }

    /// Generate one chunk through the full two-stage pipeline, the way
    /// `ChunkProviderServer.loadChunk` does: generate, insert, then let the 2×2
    /// rule populate whichever owners it completes.
    ///
    /// Only the `World`-backed callers need this; the parity harness inserts
    /// into a [`crate::TestWorld`] itself so it can keep the insertion order
    /// explicit.
    pub fn generate_into<W: GenWorld>(&mut self, world: &mut W, chunk_x: i32, chunk_z: i32) {
        let column = self.provide_chunk(chunk_x, chunk_z);
        world.insert_chunk(column);
        self.populate(world, chunk_x, chunk_z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_dimension_builds_its_own_generator() {
        for dimension in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
            let mut generator = ChunkGenerator::new(dimension, 12345);
            assert_eq!(generator.dimension(), dimension);
            let column = generator.provide_chunk(0, 0);
            assert_eq!(column.x, 0);
            assert_eq!(column.z, 0);
        }
    }

    #[test]
    fn the_nether_and_the_end_have_no_skylight() {
        assert!(!Dimension::Nether.has_skylight());
        assert!(!Dimension::End.has_skylight());
    }
}
