//! Anvil region files: 32x32 chunks in one `.mca`, 4 KiB sectors, zlib payloads.

use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;

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

    /// The whole compressed record, excluding sector padding.
    fn record(&self, x: usize, z: usize) -> Result<Option<&[u8]>, RegionError> {
        if x >= CHUNKS_PER_SIDE || z >= CHUNKS_PER_SIDE {
            return Err(RegionError::BadChunkIndex { x, z });
        }
        let (offset, sectors) = self.location(x, z);
        if (offset, sectors) == (0, 0) {
            return Ok(None);
        }
        let invalid = || RegionError::BadOffset { x, z, offset };
        if offset < HEADER_SECTORS || sectors == 0 || offset + sectors > self.data.len() / SECTOR {
            return Err(invalid());
        }
        let allocated = &self.data[offset * SECTOR..(offset + sectors) * SECTOR];
        let length = u32::from_be_bytes(allocated[..4].try_into().unwrap()) as usize;
        if length == 0 || length > allocated.len() - 4 {
            return Err(invalid());
        }
        Ok(Some(&allocated[..4 + length]))
    }

    /// Decompressed NBT of one chunk, or None when that slot is empty.
    pub fn chunk_nbt(&self, x: usize, z: usize) -> Result<Option<Vec<u8>>, RegionError> {
        let Some(record) = self.record(x, z)? else {
            return Ok(None);
        };
        let kind = record[4];
        let payload = &record[5..];
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

    pub fn write_chunk(
        path: impl AsRef<Path>,
        x: usize,
        z: usize,
        nbt: &[u8],
    ) -> Result<(), RegionError> {
        Self::write_chunks(path, &[(x, z, nbt)])
    }

    /// Pack the live records into a new region. The old header and all records
    /// stay intact until the completed sibling file replaces them together.
    pub fn write_chunks<B: AsRef<[u8]>>(
        path: impl AsRef<Path>,
        updates: &[(usize, usize, B)],
    ) -> Result<(), RegionError> {
        if updates.is_empty() {
            return Ok(());
        }
        let path = path.as_ref();
        let io_error = |source| RegionError::Io {
            path: path.to_owned(),
            source,
        };
        let mut slots = [None; CHUNKS_PER_SIDE * CHUNKS_PER_SIDE];
        for (x, z, bytes) in updates {
            if *x >= CHUNKS_PER_SIDE || *z >= CHUNKS_PER_SIDE {
                return Err(RegionError::BadChunkIndex { x: *x, z: *z });
            }
            slots[x + z * CHUNKS_PER_SIDE] = Some(bytes.as_ref());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let old = match Self::open(path) {
            Ok(region) => Some(region),
            Err(RegionError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let temporary =
            path.with_extension(format!("mca.{}-{}.tmp", std::process::id(), now.as_nanos()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io_error)?;
        let result = (|| {
            let mut header = old.as_ref().map_or([0; SECTOR * 2], |region| region.header);
            file.write_all(&header).map_err(io_error)?;
            let mut offset = HEADER_SECTORS;
            for (index, update) in slots.into_iter().enumerate() {
                let (x, z) = (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE);
                let mut compressed;
                let record = if let Some(nbt) = update {
                    let mut encoder = ZlibEncoder::new(vec![0; 5], Compression::default());
                    encoder.write_all(nbt).map_err(io_error)?;
                    compressed = encoder.finish().map_err(io_error)?;
                    let length = compressed.len();
                    if length.div_ceil(SECTOR) > u8::MAX as usize {
                        return Err(RegionError::ChunkTooLarge { x, z, length });
                    }
                    compressed[..4].copy_from_slice(&((length - 4) as u32).to_be_bytes());
                    compressed[4] = 2;
                    let stamp = now.as_secs().min(u32::MAX as u64) as u32;
                    let time = SECTOR + index * 4;
                    header[time..time + 4].copy_from_slice(&stamp.to_be_bytes());
                    &compressed[..]
                } else if let Some(record) = old
                    .as_ref()
                    .map(|region| region.record(x, z))
                    .transpose()?
                    .flatten()
                {
                    record
                } else {
                    continue;
                };
                let count = record.len().div_ceil(SECTOR);
                let location = ((offset as u32) << 8) | count as u32;
                header[index * 4..index * 4 + 4].copy_from_slice(&location.to_be_bytes());
                file.write_all(record).map_err(io_error)?;
                let padding = count * SECTOR - record.len();
                file.write_all(&[0; SECTOR][..padding]).map_err(io_error)?;
                offset += count;
            }
            file.rewind()
                .and_then(|_| file.write_all(&header))
                .and_then(|_| file.sync_all())
                .map_err(io_error)?;
            drop(file);
            fs::rename(&temporary, path).map_err(io_error)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
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

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nephele-anvil-region-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[cfg(windows)]
    #[test]
    fn windows_replace_failure_can_retry_without_losing_the_previous_region() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = test_dir();
        let path = dir.join("r.0.0.mca");
        RegionFile::write_chunk(&path, 0, 0, b"old").unwrap();
        let before = fs::read(&path).unwrap();
        // Hold the primary through the publish step, denying FILE_SHARE_DELETE.
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(RegionFile::write_chunk(&path, 0, 0, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            RegionFile::open(&path)
                .unwrap()
                .chunk_nbt(0, 0)
                .unwrap()
                .unwrap(),
            b"old"
        );
        drop(lock);
        RegionFile::write_chunk(&path, 0, 0, b"new").unwrap();
        assert_eq!(
            RegionFile::open(&path)
                .unwrap()
                .chunk_nbt(0, 0)
                .unwrap()
                .unwrap(),
            b"new"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compaction_keeps_the_reference_worlds_untouched_records_and_timestamps() {
        let Some(original) = reference_region() else {
            return;
        };
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.-1.0.mca");
        fs::copy(original, &path).unwrap();
        let before = RegionFile::open(&path).unwrap();
        let slots = before.present();
        let (x, z) = slots[0];
        let updated = before.chunk_nbt(x, z).unwrap().unwrap();
        RegionFile::write_chunk(&path, x, z, &updated).unwrap();
        let after = RegionFile::open(&path).unwrap();
        assert_eq!(after.present(), slots);
        assert_eq!(after.chunk_nbt(x, z).unwrap().unwrap(), updated);
        let stamp = SECTOR + 4 * (x + CHUNKS_PER_SIDE * z);
        assert_ne!(after.header[stamp..stamp + 4], [0; 4]);
        for &(x, z) in &slots[1..] {
            assert_eq!(after.record(x, z).unwrap(), before.record(x, z).unwrap());
            let stamp = SECTOR + 4 * (x + CHUNKS_PER_SIDE * z);
            assert_eq!(
                after.header[stamp..stamp + 4],
                before.header[stamp..stamp + 4]
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_record_cannot_read_beyond_its_sector_into_its_neighbor() {
        let dir = test_dir();
        let path = dir.join("r.0.0.mca");
        RegionFile::write_chunks(&path, &[(0, 0, b"abc"), (1, 0, b"def")]).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[2 * SECTOR..2 * SECTOR + 4].copy_from_slice(&(SECTOR as u32).to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        let region = RegionFile::open(&path).unwrap();
        assert!(matches!(
            region.chunk_nbt(0, 0),
            Err(RegionError::BadOffset { .. })
        ));
        assert!(RegionFile::write_chunk(&path, 1, 0, b"another").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writing_a_chunk_preserves_other_slots_and_reopens() {
        let dir = test_dir();
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

        let mut random = 1u32;
        let noise: Vec<u8> = (0..SECTOR * 5)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                random as u8
            })
            .collect();
        for size in [noise.len(), 12, noise.len(), 12] {
            RegionFile::write_chunk(&path, 3, 5, &noise[..size]).unwrap();
            let region = RegionFile::open(&path).unwrap();
            assert_eq!(region.chunk_nbt(3, 5).unwrap().unwrap(), noise[..size]);
            assert_eq!(
                region.chunk_nbt(31, 2).unwrap(),
                before.chunk_nbt(31, 2).unwrap()
            );
            assert_eq!(region.data.len() / SECTOR, if size == 12 { 4 } else { 9 });
        }
        let _ = fs::remove_dir_all(dir);
    }
}
