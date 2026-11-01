//! Deterministic flat-ish terrain so the mock server always sends the exact
//! same world. Not a world generator — just enough blocks to exercise the
//! decoder and later the renderer.

use nephele_world::{BlockState, ChunkColumn};

pub const SEA_LEVEL: usize = 64;

/// Gentle hills derived from sine waves; deterministic and chunk-independent,
/// so neighbouring chunks line up seamlessly.
pub fn height_at(world_x: i32, world_z: i32) -> usize {
    let x = world_x as f64;
    let z = world_z as f64;
    let a = (x * 0.11).sin();
    let b = (z * 0.09).cos();
    let c = ((x + z) * 0.045).sin();
    let height = SEA_LEVEL as f64 + (a + b) * 2.4 + c * 1.6;
    height.round().clamp(4.0, 120.0) as usize
}

/// True where a tree trunk belongs. Kept away from chunk borders so a tree
/// never spills into a neighbouring column.
fn has_tree(world_x: i32, world_z: i32, local_x: usize, local_z: usize) -> bool {
    if !(3..=12).contains(&local_x) || !(3..=12).contains(&local_z) {
        return false;
    }
    let noise = (world_x.wrapping_mul(31) ^ world_z.wrapping_mul(17)).rem_euclid(97);
    noise == 0
}

pub fn generate_column(chunk_x: i32, chunk_z: i32) -> ChunkColumn {
    let mut column = ChunkColumn::empty(chunk_x, chunk_z);

    for local_x in 0..16usize {
        for local_z in 0..16usize {
            let world_x = chunk_x * 16 + local_x as i32;
            let world_z = chunk_z * 16 + local_z as i32;
            let height = height_at(world_x, world_z);

            let surface = if height > SEA_LEVEL + 8 {
                BlockState::SNOW_BLOCK
            } else if height + 2 <= SEA_LEVEL {
                BlockState::SAND
            } else {
                BlockState::GRASS
            };

            for y in 0..=height {
                let block = if y == 0 {
                    BlockState::BEDROCK
                } else if y == height {
                    surface
                } else if y + 3 >= height {
                    if surface == BlockState::SAND {
                        BlockState::SAND
                    } else {
                        BlockState::DIRT
                    }
                } else {
                    BlockState::STONE
                };
                column.set_block(local_x, y, local_z, block);
            }

            // Fill low ground with water up to sea level.
            for y in (height + 1)..=SEA_LEVEL {
                column.set_block(local_x, y, local_z, BlockState::WATER);
            }

            if has_tree(world_x, world_z, local_x, local_z) {
                let trunk_top = height + 5;
                for y in (height + 1)..=trunk_top {
                    column.set_block(local_x, y, local_z, BlockState::LOG);
                }
                for dx in -2i32..=2 {
                    for dz in -2i32..=2 {
                        for dy in 0..=1 {
                            if dx == 0 && dz == 0 {
                                continue;
                            }
                            if dx.abs() == 2 && dz.abs() == 2 {
                                continue;
                            }
                            let lx = local_x as i32 + dx;
                            let lz = local_z as i32 + dz;
                            if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                                continue;
                            }
                            let y = trunk_top - 1 + dy;
                            if column.get_block(lx as usize, y, lz as usize).is_air() {
                                column.set_block(lx as usize, y, lz as usize, BlockState::LEAVES);
                            }
                        }
                    }
                }
            }
        }
    }

    // Open sky: full skylight in every generated section, no block light.
    for index in 0..nephele_world::SECTION_COUNT {
        if let Some(section) = column.section_mut(index) {
            section.set_sky_light(&[0xFF; nephele_world::chunk::SECTION_LIGHT_BYTES]);
        }
    }
    column.set_biomes(&[1u8; 256]);
    column
}

/// Generate the square of columns around the origin that the mock server sends
/// on join.
pub fn generate_region(radius: i32) -> Vec<ChunkColumn> {
    let mut columns = Vec::new();
    for cx in -radius..=radius {
        for cz in -radius..=radius {
            columns.push(generate_column(cx, cz));
        }
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_is_deterministic() {
        let a = generate_column(0, 0);
        let b = generate_column(0, 0);
        assert_eq!(a.non_air_count(), b.non_air_count());
        assert_eq!(a.mask(), b.mask());
        for y in 0..80 {
            assert_eq!(a.get_block(4, y, 4), b.get_block(4, y, 4));
        }
    }

    #[test]
    fn surface_matches_the_biome_rules() {
        let column = generate_column(2, 3);
        let height = height_at(2 * 16 + 4, 3 * 16 + 4);
        let top = column.get_block(4, height, 4);
        assert!(
            [BlockState::GRASS, BlockState::SAND, BlockState::SNOW_BLOCK].contains(&top),
            "unexpected surface {top:?} at height {height}"
        );
        if top == BlockState::GRASS {
            assert_eq!(column.get_block(4, height - 1, 4), BlockState::DIRT);
        }
        assert_eq!(column.get_block(4, 0, 4), BlockState::BEDROCK);
        let above = column.get_block(4, height + 1, 4);
        assert!(
            above.is_air() || above == BlockState::WATER,
            "above = {above:?}"
        );
    }

    #[test]
    fn low_ground_is_flooded_to_sea_level() {
        // Find a column below sea level and check it holds water above ground.
        let mut found = false;
        for wx in -40..40 {
            for wz in -40..40 {
                let height = height_at(wx, wz);
                if height + 2 <= SEA_LEVEL {
                    let column = generate_column(wx.div_euclid(16), wz.div_euclid(16));
                    let x = wx.rem_euclid(16) as usize;
                    let z = wz.rem_euclid(16) as usize;
                    assert_eq!(column.get_block(x, SEA_LEVEL, z), BlockState::WATER);
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(
            found,
            "test terrain should have a low area; adjust the search range"
        );
    }

    #[test]
    fn neighbouring_columns_agree_on_border_heights() {
        let west = generate_column(-1, 0);
        let east = generate_column(0, 0);
        for z in 0..16 {
            let west_edge = height_at(-1, z as i32);
            let east_edge = height_at(0, z as i32);
            assert_eq!(west.get_block(15, west_edge, z), BlockState::GRASS);
            assert_eq!(east.get_block(0, east_edge, z), BlockState::GRASS);
        }
    }
}
