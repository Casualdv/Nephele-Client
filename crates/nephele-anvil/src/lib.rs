//! `nephele-anvil` - reading the game's own world save format.
//!
//! Anvil is what 1.8.9 writes to disk: `.mca` region files of 32x32 chunks,
//! each chunk a zlib-compressed NBT tree. Nephele uses it for two things:
//! loading worlds later, and checking its generator against a world the real
//! game produced.
//!
//! Nothing here ships game content: a region file comes from the player's own
//! world and never enters the repository.

pub mod chunk;
pub mod nbt;
pub mod region;

pub use chunk::{
    encode_chunk_nbt, parse_chunk, ChunkError, StoredChunk, StoredSection, SECTION_VOLUME,
};
pub use nbt::{encode as encode_nbt, parse as parse_nbt, NbtError, Tag};
pub use region::{RegionError, RegionFile};

/// World-space chunk coordinates of region `(region_x, region_z)` plus local
/// `(local_x, local_z)`, both in 0..32.
pub fn chunk_coords(region_x: i32, region_z: i32, local_x: usize, local_z: usize) -> (i32, i32) {
    (
        region_x * 32 + local_x as i32,
        region_z * 32 + local_z as i32,
    )
}

/// Parses `r.<x>.<z>.mca` into region coordinates.
pub fn region_coords(path: &std::path::Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_prefix('r')?
        .strip_suffix(".mca")?
        .trim_start_matches('.');
    let mut parts = stem.split('.');
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some((x, z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn region_names_parse_both_signs() {
        assert_eq!(region_coords(Path::new("r.0.0.mca")), Some((0, 0)));
        assert_eq!(region_coords(Path::new("r.-1.0.mca")), Some((-1, 0)));
        assert_eq!(region_coords(Path::new("r.-1.-2.mca")), Some((-1, -2)));
        assert_eq!(region_coords(Path::new("r.12.34.mca")), Some((12, 34)));
        assert_eq!(region_coords(Path::new("not-a-region.mca")), None);
    }

    #[test]
    fn chunk_coordinates_are_region_relative() {
        assert_eq!(chunk_coords(-1, 0, 0, 0), (-32, 0));
        assert_eq!(chunk_coords(-1, 0, 31, 31), (-1, 31));
        assert_eq!(chunk_coords(0, 0, 0, 0), (0, 0));
    }
}
