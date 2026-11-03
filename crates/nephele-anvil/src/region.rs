//! Anvil region files: 32x32 chunks in one `.mca`, 4 KiB sectors, zlib payloads.

use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;

const SECTOR: usize = 4096;
const HEADER_SECTORS: usize = 2;
const CHUNKS_PER_SIDE: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("region file {path} is {size} bytes, too small for its header")]
    Truncated { path: PathBuf, size: usize },
    #[error("chunk {x},{z}: sector offset {offset} lies outside the file")]
    BadOffset { x: usize, z: usize, offset: usize },
    #[error("chunk {x},{z} uses unknown compression {kind}")]
    UnknownCompression { x: usize, z: usize, kind: u8 },
    #[error("chunk {x},{z} failed to inflate: {source}")]
    Inflate {
        x: usize,
        z: usize,
        #[source]
        source: std::io::Error,
    },
    #[error("chunk index {x},{z} is outside the 0..32 region bounds")]
    BadChunkIndex { x: usize, z: usize },
    #[error("chunk {x},{z} is too large for an Anvil region ({length} bytes)")]
    ChunkTooLarge { x: usize, z: usize, length: usize },
    #[error("region file {path} needs a sector offset larger than 24 bits")]
    RegionTooLarge { path: PathBuf },
}

/// One `.mca` region, read into memory.
pub struct RegionFile {
    path: PathBuf,
    data: Vec<u8>,
    header: [u8; HEADER_SECTORS * SECTOR],
}

impl RegionFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegionError> {
        let path = path.as_ref().to_path_buf();
        let data = fs::read(&path).map_err(|source| RegionError::Io {
            path: path.clone(),
            source,
        })?;
        if data.len() < HEADER_SECTORS * SECTOR {
            return Err(RegionError::Truncated {
                path,
                size: data.len(),
            });
        }
        let mut header = [0u8; HEADER_SECTORS * SECTOR];
        header.copy_from_slice(&data[..HEADER_SECTORS * SECTOR]);
        Ok(Self { path, data, header })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn location(&self, x: usize, z: usize) -> (usize, usize) {
        let index = (x & 31) + (z & 31) * CHUNKS_PER_SIDE;
        let base = index * 4;
        let offset = ((self.header[base] as usize) << 16)
            | ((self.header[base + 1] as usize) << 8)
            | (self.header[base + 2] as usize);
        let sectors = self.header[base + 3] as usize;
        (offset, sectors)
    }

    /// Every chunk index present in this region, as local `(x, z)` in 0..32.
    pub fn present(&self) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for z in 0..CHUNKS_PER_SIDE {
            for x in 0..CHUNKS_PER_SIDE {
                if self.location(x, z).0 != 0 {
                    found.push((x, z));
                }
            }
        }
        found
    }

    /// Decompressed NBT of one chunk, or `None` when that slot is empty.
    pub fn chunk_nbt(&self, x: usize, z: usize) -> Result<Option<Vec<u8>>, RegionError> {
        let (offset, sectors) = self.location(x, z);
        if offset == 0 || sectors == 0 {
            return Ok(None);
        }
        let start = offset * SECTOR;
        if start + 5 > self.data.len() {
            return Err(RegionError::BadOffset { x, z, offset });
        }
        let length = u32::from_be_bytes(self.data[start..start + 4].try_into().unwrap()) as usize;
        if length < 1 || start + 4 + length > self.data.len() {
            return Err(RegionError::BadOffset { x, z, offset });
        }
        let kind = self.data[start + 4];
        let payload = &self.data[start + 5..start + 4 + length];
        let mut out = Vec::new();
        match kind {
            1 => {
                GzDecoder::new(payload)
                    .read_to_end(&mut out)
                    .map_err(|source| RegionError::Inflate { x, z, source })?;
            }
            2 => {
                ZlibDecoder::new(payload)
                    .read_to_end(&mut out)
                    .map_err(|source| RegionError::Inflate { x, z, source })?;
            }
            3 => out.extend_from_slice(payload),
            other => return Err(RegionError::UnknownCompression { x, z, kind: other }),
        }
        Ok(Some(out))
    }

    /// Appends a zlib-compressed NBT chunk and switches its location-table
    /// entry to the new sectors. Unmodified chunks keep their existing sectors.
    pub fn write_chunk(
        path: impl AsRef<Path>,
        x: usize,
        z: usize,
        nbt: &[u8],
    ) -> Result<(), RegionError> {
        let path = path.as_ref().to_path_buf();
        if x >= CHUNKS_PER_SIDE || z >= CHUNKS_PER_SIDE {
            return Err(RegionError::BadChunkIndex { x, z });
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;
        }

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(nbt).map_err(|source| RegionError::Io {
            path: path.clone(),
            source,
        })?;
        let compressed = encoder.finish().map_err(|source| RegionError::Io {
            path: path.clone(),
            source,
        })?;
        let length = compressed
            .len()
            .checked_add(1)
            .ok_or(RegionError::ChunkTooLarge {
                x,
                z,
                length: compressed.len(),
            })?;
        let length_u32 =
            u32::try_from(length).map_err(|_| RegionError::ChunkTooLarge { x, z, length })?;
        let record_length =
            length
                .checked_add(4)
                .ok_or(RegionError::ChunkTooLarge { x, z, length })?;
        let sectors = record_length.div_ceil(SECTOR);
        if sectors == 0 || sectors > u8::MAX as usize {
            return Err(RegionError::ChunkTooLarge {
                x,
                z,
                length: record_length,
            });
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;
        let file_length = file
            .metadata()
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?
            .len();
        if file_length == 0 {
            file.set_len((HEADER_SECTORS * SECTOR) as u64)
                .map_err(|source| RegionError::Io {
                    path: path.clone(),
                    source,
                })?;
        } else if file_length < (HEADER_SECTORS * SECTOR) as u64 {
            return Err(RegionError::Truncated {
                path,
                size: file_length as usize,
            });
        }

        let index = x + z * CHUNKS_PER_SIDE;
        let mut old_location_bytes = [0u8; 4];
        file.seek(SeekFrom::Start((index * 4) as u64))
            .and_then(|_| file.read_exact(&mut old_location_bytes))
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;
        let old_location = u32::from_be_bytes(old_location_bytes);
        let old_start_sector = (old_location >> 8) as u64;
        let old_sectors = (old_location & 0xFF) as usize;
        let current_length = file
            .metadata()
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?
            .len();
        let current_sectors = current_length.div_ceil(SECTOR as u64);
        let can_reuse = old_start_sector >= HEADER_SECTORS as u64
            && old_sectors >= sectors
            && old_start_sector + old_sectors as u64 <= current_sectors;
        let (start_sector, allocated_sectors) = if can_reuse {
            (old_start_sector, old_sectors)
        } else {
            (current_sectors.max(HEADER_SECTORS as u64), sectors)
        };
        if start_sector > 0x00FF_FFFF {
            return Err(RegionError::RegionTooLarge { path });
        }
        let start = start_sector * SECTOR as u64;
        let mut record = Vec::with_capacity(allocated_sectors * SECTOR);
        record.extend_from_slice(&length_u32.to_be_bytes());
        record.push(2); // zlib
        record.extend_from_slice(&compressed);
        record.resize(allocated_sectors * SECTOR, 0);

        file.seek(SeekFrom::Start(start))
            .and_then(|_| file.write_all(&record))
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;
        file.sync_data().map_err(|source| RegionError::Io {
            path: path.clone(),
            source,
        })?;

        let location = ((start_sector as u32) << 8) | allocated_sectors as u32;
        let location_offset = (index * 4) as u64;
        file.seek(SeekFrom::Start(location_offset))
            .and_then(|_| file.write_all(&location.to_be_bytes()))
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(u32::MAX as u64) as u32;
        let timestamp_offset = (SECTOR + index * 4) as u64;
        file.seek(SeekFrom::Start(timestamp_offset))
            .and_then(|_| file.write_all(&timestamp.to_be_bytes()))
            .map_err(|source| RegionError::Io {
                path: path.clone(),
                source,
            })?;
        file.sync_data()
            .map_err(|source| RegionError::Io { path, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference world generated by the real 1.8.9 server, if it is here.
    fn reference_region() -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Nephele-private/parity/server/world/region/r.-1.0.mca");
        path.is_file().then_some(path)
    }

    #[test]
    fn reads_the_reference_region() {
        let Some(path) = reference_region() else {
            eprintln!("skipped: no reference world at Nephele-private/parity");
            return;
        };
        let region = RegionFile::open(&path).expect("region opens");
        let present = region.present();
        assert!(
            present.len() > 100,
            "expected a populated region, got {} chunks",
            present.len()
        );

        let (x, z) = present[0];
        let nbt = region
            .chunk_nbt(x, z)
            .expect("chunk reads")
            .expect("present");
        assert!(
            nbt.len() > 1000,
            "chunk NBT looks truncated: {} bytes",
            nbt.len()
        );
    }

    #[test]
    fn empty_slots_return_none() {
        let Some(path) = reference_region() else {
            eprintln!("skipped: no reference world");
            return;
        };
        let region = RegionFile::open(&path).expect("region opens");
        // Most slots in a spawn-only region are empty.
        let empty = (0..32)
            .flat_map(|z| (0..32).map(move |x| (x, z)))
            .filter(|(x, z)| region.location(*x, *z).0 == 0)
            .count();
        assert!(empty > 0, "a spawn-only region should have gaps");
    }

    #[test]
    fn writing_a_chunk_preserves_other_slots_and_reopens() {
        let dir = std::env::temp_dir().join(format!(
            "nephele-anvil-region-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("region/r.-1.0.mca");

        RegionFile::write_chunk(&path, 31, 2, b"hand edited west chunk").unwrap();
        RegionFile::write_chunk(&path, 3, 5, b"neighbour chunk").unwrap();
        let before = RegionFile::open(&path).unwrap();
        let west_offset = before.location(31, 2).0;
        let length_before_update = fs::metadata(&path).unwrap().len();
        assert_eq!(
            before.chunk_nbt(31, 2).unwrap().as_deref(),
            Some(&b"hand edited west chunk"[..])
        );

        RegionFile::write_chunk(&path, 3, 5, b"updated neighbour").unwrap();
        let after = RegionFile::open(&path).unwrap();
        assert_eq!(after.location(31, 2).0, west_offset);
        assert_eq!(
            after.chunk_nbt(31, 2).unwrap().as_deref(),
            Some(&b"hand edited west chunk"[..])
        );
        assert_eq!(
            after.chunk_nbt(3, 5).unwrap().as_deref(),
            Some(&b"updated neighbour"[..])
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), length_before_update);

        let _ = fs::remove_dir_all(dir);
    }
}
