//! Prints what a saved chunk actually contains: biome ids, column heights and
//! the block at the top of each column. Used to check the generator port against
//! a world the real game produced.
//!
//! ```text
//! cargo run -p nephele-anvil --example inspect -- <region.mca> <local_x> <local_z>
//! ```

use std::collections::BTreeMap;

use nephele_anvil::{parse_chunk, RegionFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: inspect <region.mca> <local_x> <local_z>")?;
    let local_x: usize = args.next().ok_or("missing local x")?.parse()?;
    let local_z: usize = args.next().ok_or("missing local z")?.parse()?;

    let region = RegionFile::open(&path)?;
    let Some(bytes) = region.chunk_nbt(local_x, local_z)? else {
        eprintln!("chunk slot {local_x},{local_z} is empty");
        return Ok(());
    };
    let chunk = parse_chunk(&bytes)?;

    println!("chunk ({}, {})", chunk.x, chunk.z);
    println!("sections: {}", chunk.sections.len());

    let mut biomes: BTreeMap<u8, usize> = BTreeMap::new();
    for id in &chunk.biomes {
        *biomes.entry(*id).or_insert(0) += 1;
    }
    println!("biomes: {biomes:?}");

    let mut heights = [[0usize; 16]; 16];
    for (z, row) in heights.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = chunk.height(x, z).map(|h| h + 1).unwrap_or(0);
        }
    }
    println!("column tops (y of first air):");
    for row in heights {
        let line: Vec<String> = row.iter().map(|h| format!("{h:3}")).collect();
        println!("  {}", line.join(" "));
    }

    println!("biome map (x fastest):");
    for z in 0..16 {
        let line: Vec<String> = (0..16)
            .map(|x| format!("{:3}", chunk.biome(x, z).unwrap_or(255)))
            .collect();
        println!("  {}", line.join(" "));
    }
    Ok(())
}
