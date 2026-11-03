//! Single-player world metadata and Anvil chunk persistence.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nephele_anvil::{
    encode_chunk_nbt, encode_nbt, parse_chunk, parse_nbt, RegionFile, StoredChunk, StoredSection,
    Tag,
};

use crate::chunk::{ChunkColumn, Section, SECTION_COUNT, SECTION_VOLUME};

const METADATA_FILE: &str = "nephele.dat";
const REGION_DIR: &str = "region";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldMode {
    Creative,
    Survival,
}

impl WorldMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creative => "creative",
            Self::Survival => "survival",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "creative" => Some(Self::Creative),
            "survival" => Some(Self::Survival),
            _ => None,
        }
    }

    pub const fn is_creative(self) -> bool {
        matches!(self, Self::Creative)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldMetadata {
    /// Directory name under the `saves` folder; stable even if the display name changes.
    pub directory: String,
    pub name: String,
    pub seed: u64,
    pub mode: WorldMode,
}

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    Region(nephele_anvil::RegionError),
    Chunk(nephele_anvil::ChunkError),
    Nbt(nephele_anvil::NbtError),
    InvalidWorldName(String),
    AlreadyExists(PathBuf),
    InvalidMetadata(PathBuf),
    ChunkCoordinateMismatch {
        requested: (i32, i32),
        stored: (i32, i32),
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "world save I/O error: {error}"),
            Self::Region(error) => write!(f, "Anvil region error: {error}"),
            Self::Chunk(error) => write!(f, "Anvil chunk error: {error}"),
            Self::Nbt(error) => write!(f, "world metadata NBT error: {error}"),
            Self::InvalidWorldName(name) => write!(f, "invalid world name: {name:?}"),
            Self::AlreadyExists(path) => {
                write!(f, "a world already exists at {}", path.display())
            }
            Self::InvalidMetadata(path) => {
                write!(
                    f,
                    "invalid or unsupported world metadata at {}",
                    path.display()
                )
            }
            Self::ChunkCoordinateMismatch { requested, stored } => write!(
                f,
                "saved chunk coordinates {},{} do not match requested {},{}",
                stored.0, stored.1, requested.0, requested.1
            ),
        }
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Region(error) => Some(error),
            Self::Chunk(error) => Some(error),
            Self::Nbt(error) => Some(error),
            Self::InvalidWorldName(_)
            | Self::AlreadyExists(_)
            | Self::InvalidMetadata(_)
            | Self::ChunkCoordinateMismatch { .. } => None,
        }
    }
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<nephele_anvil::RegionError> for SaveError {
    fn from(error: nephele_anvil::RegionError) -> Self {
        Self::Region(error)
    }
}

impl From<nephele_anvil::ChunkError> for SaveError {
    fn from(error: nephele_anvil::ChunkError) -> Self {
        Self::Chunk(error)
    }
}

impl From<nephele_anvil::NbtError> for SaveError {
    fn from(error: nephele_anvil::NbtError) -> Self {
        Self::Nbt(error)
    }
}

/// A local world and its cached region readers.
pub struct WorldSave {
    path: PathBuf,
    metadata: WorldMetadata,
    regions: HashMap<(i32, i32), RegionFile>,
}

impl WorldSave {
    /// Creates a save at `<saves_dir>/<name>` and writes its metadata.
    pub fn create(
        saves_dir: impl AsRef<Path>,
        name: &str,
        seed: u64,
        mode: WorldMode,
    ) -> Result<Self, SaveError> {
        let name = name.trim();
        let directory = validate_directory_name(name)?;
        let path = saves_dir.as_ref().join(&directory);
        fs::create_dir_all(saves_dir.as_ref())?;
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(SaveError::AlreadyExists(path));
            }
            Err(error) => return Err(error.into()),
        }
        fs::create_dir(path.join(REGION_DIR))?;

        let metadata = WorldMetadata {
            directory,
            name: name.to_string(),
            seed,
            mode,
        };
        write_metadata(&path.join(METADATA_FILE), &metadata)?;
        Ok(Self {
            path,
            metadata,
            regions: HashMap::new(),
        })
    }

    /// Opens an existing save by its directory name.
    pub fn open(saves_dir: impl AsRef<Path>, directory: &str) -> Result<Self, SaveError> {
        let directory = validate_directory_name(directory)?;
        let path = saves_dir.as_ref().join(&directory);
        let metadata_path = path.join(METADATA_FILE);
        let mut metadata = read_metadata(&metadata_path)?;
        metadata.directory = directory;
        Ok(Self {
            path,
            metadata,
            regions: HashMap::new(),
        })
    }

    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    /// Persists a changed mode in the world's metadata.
    pub fn set_mode(&mut self, mode: WorldMode) -> Result<(), SaveError> {
        if self.metadata.mode == mode {
            return Ok(());
        }
        let previous = self.metadata.mode;
        self.metadata.mode = mode;
        if let Err(error) = write_metadata(&self.path.join(METADATA_FILE), &self.metadata) {
            self.metadata.mode = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Loads a previously saved chunk, or returns `None` for a generated-only chunk.
    pub fn load_chunk(&mut self, x: i32, z: i32) -> Result<Option<ChunkColumn>, SaveError> {
        let region_coord = (x.div_euclid(32), z.div_euclid(32));
        let region_path = self.region_path(region_coord);
        if !region_path.exists() {
            return Ok(None);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.regions.entry(region_coord) {
            entry.insert(RegionFile::open(&region_path)?);
        }

        let local_x = x.rem_euclid(32) as usize;
        let local_z = z.rem_euclid(32) as usize;
        let Some(bytes) = self
            .regions
            .get(&region_coord)
            .expect("region inserted above")
            .chunk_nbt(local_x, local_z)?
        else {
            return Ok(None);
        };
        let stored = parse_chunk(&bytes)?;
        if (stored.x, stored.z) != (x, z) {
            return Err(SaveError::ChunkCoordinateMismatch {
                requested: (x, z),
                stored: (stored.x, stored.z),
            });
        }
        Ok(Some(column_from_stored(stored)))
    }

    /// Writes exactly this chunk to its region file. Callers decide which chunks
    /// are dirty; clean chunks are never serialized or rewritten here.
    pub fn save_chunk(&mut self, column: &ChunkColumn) -> Result<(), SaveError> {
        let region_coord = (column.x.div_euclid(32), column.z.div_euclid(32));
        let region_path = self.region_path(region_coord);
        let bytes = encode_chunk_nbt(&stored_from_column(column))?;
        RegionFile::write_chunk(
            &region_path,
            column.x.rem_euclid(32) as usize,
            column.z.rem_euclid(32) as usize,
            &bytes,
        )?;
        self.regions.remove(&region_coord);
        Ok(())
    }

    fn region_path(&self, (x, z): (i32, i32)) -> PathBuf {
        self.path.join(REGION_DIR).join(format!("r.{x}.{z}.mca"))
    }
}

/// Lists readable world saves in stable display-name order.
pub fn list_worlds(saves_dir: impl AsRef<Path>) -> Result<Vec<WorldMetadata>, SaveError> {
    let saves_dir = saves_dir.as_ref();
    if !saves_dir.exists() {
        return Ok(Vec::new());
    }

    let mut worlds = Vec::new();
    for entry in fs::read_dir(saves_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(directory) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_directory_name(&directory).is_err() {
            continue;
        }
        let metadata_path = entry.path().join(METADATA_FILE);
        if let Ok(mut metadata) = read_metadata(&metadata_path) {
            metadata.directory = directory;
            worlds.push(metadata);
        }
    }
    worlds.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.directory.cmp(&right.directory))
    });
    Ok(worlds)
}

fn validate_directory_name(name: &str) -> Result<String, SaveError> {
    let invalid = name.is_empty()
        || name.len() > 96
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        });
    let device_name = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (1..=9).any(|index| {
            device_name == format!("COM{index}") || device_name == format!("LPT{index}")
        });
    if invalid || reserved {
        return Err(SaveError::InvalidWorldName(name.to_string()));
    }
    Ok(name.to_string())
}

fn write_metadata(path: &Path, metadata: &WorldMetadata) -> Result<(), SaveError> {
    let mut values = HashMap::new();
    values.insert("Name".to_string(), Tag::String(metadata.name.clone()));
    values.insert("Seed".to_string(), Tag::Long(metadata.seed as i64));
    values.insert(
        "Mode".to_string(),
        Tag::String(metadata.mode.as_str().to_string()),
    );
    values.insert("Version".to_string(), Tag::Int(1));
    let bytes = encode_nbt("Nephele", &Tag::Compound(values))?;
    fs::write(path, bytes)?;
    Ok(())
}

fn read_metadata(path: &Path) -> Result<WorldMetadata, SaveError> {
    let bytes = fs::read(path)?;
    let (_, root) = parse_nbt(&bytes)?;
    let name = match root.get("Name") {
        Some(Tag::String(name)) if !name.trim().is_empty() => name.clone(),
        _ => return Err(SaveError::InvalidMetadata(path.to_path_buf())),
    };
    let seed = match root.get("Seed") {
        Some(Tag::Long(seed)) => *seed as u64,
        _ => return Err(SaveError::InvalidMetadata(path.to_path_buf())),
    };
    let mode = match root.get("Mode") {
        Some(Tag::String(mode)) => WorldMode::parse(mode),
        _ => None,
    }
    .ok_or_else(|| SaveError::InvalidMetadata(path.to_path_buf()))?;

    Ok(WorldMetadata {
        directory: String::new(),
        name,
        seed,
        mode,
    })
}

fn stored_from_column(column: &ChunkColumn) -> StoredChunk {
    let sections = column
        .iter_sections()
        .map(|(y, section)| {
            let mut blocks = [0u8; SECTION_VOLUME];
            let mut add = [0u8; SECTION_VOLUME / 2];
            let mut data = [0u8; SECTION_VOLUME / 2];
            let mut has_add = false;

            for (index, raw) in section.blocks().iter().copied().enumerate() {
                let id = raw >> 4;
                blocks[index] = id as u8;
                let high_id = ((id >> 8) & 0x0F) as u8;
                if high_id != 0 {
                    has_add = true;
                }
                set_nibble(&mut add, index, high_id);
                set_nibble(&mut data, index, (raw & 0x0F) as u8);
            }

            let mut block_light = [0u8; SECTION_VOLUME / 2];
            block_light.copy_from_slice(section.block_light());
            let sky_light = section.sky_light().map(|bytes| {
                let mut packed = [0u8; SECTION_VOLUME / 2];
                packed.copy_from_slice(bytes);
                packed
            });

            StoredSection {
                y: y as i8,
                blocks,
                add: has_add.then_some(add),
                data: Some(data),
                block_light: Some(block_light),
                sky_light,
            }
        })
        .collect();
    let biomes = column
        .biomes()
        .map(<[u8]>::to_vec)
        .unwrap_or_else(|| vec![1; 256]);
    StoredChunk {
        x: column.x,
        z: column.z,
        sections,
        biomes,
    }
}

fn column_from_stored(stored: StoredChunk) -> ChunkColumn {
    let mut sections: Vec<Option<Section>> = vec![None; SECTION_COUNT];
    for stored_section in stored.sections {
        let Ok(y) = usize::try_from(stored_section.y) else {
            continue;
        };
        if y >= SECTION_COUNT {
            continue;
        }

        let mut blocks = Vec::with_capacity(SECTION_VOLUME);
        let mut non_air = 0u32;
        for index in 0..SECTION_VOLUME {
            let x = index & 15;
            let block_y = index >> 8;
            let z = (index >> 4) & 15;
            let raw = stored_section.state(x, block_y, z);
            non_air += u32::from(raw != 0);
            blocks.push(raw);
        }
        let mut section = Section::from_blocks(blocks, non_air);
        if let Some(block_light) = stored_section.block_light.as_ref() {
            section.set_block_light(block_light);
        }
        if let Some(sky_light) = stored_section.sky_light.as_ref() {
            section.set_sky_light(sky_light);
        }
        sections[y] = Some(section);
    }

    let mut column = ChunkColumn::from_sections(stored.x, stored.z, sections);
    if stored.biomes.len() == 256 {
        column.set_biomes(&stored.biomes);
    }
    column
}

fn set_nibble(packed: &mut [u8; SECTION_VOLUME / 2], index: usize, value: u8) {
    let byte = &mut packed[index / 2];
    if index & 1 == 0 {
        *byte = (*byte & 0xF0) | (value & 0x0F);
    } else {
        *byte = (*byte & 0x0F) | ((value & 0x0F) << 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockState;
    use crate::chunk::SECTION_LIGHT_BYTES;

    fn temp_saves_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nephele-world-save-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn hand_edited_chunk_roundtrips_after_reopening_world() {
        let saves = temp_saves_dir("roundtrip");
        let mut save = WorldSave::create(
            &saves,
            "Hand Edited World",
            0xFEDC_BA98_7654_3210,
            WorldMode::Survival,
        )
        .unwrap();
        let mut edited = ChunkColumn::empty(-1, 2);
        edited.set_block(15, 77, 3, BlockState::new(300, 7));
        edited.set_block(0, 64, 0, BlockState::new(5, 4));
        edited.set_biomes(&vec![6; 256]);
        if let Some(section) = edited.section_mut(4) {
            section.set_block_light(&vec![0xAB; SECTION_LIGHT_BYTES]);
            section.set_sky_light(&vec![0xFF; SECTION_LIGHT_BYTES]);
        }
        save.save_chunk(&edited).unwrap();
        drop(save);

        let mut reopened = WorldSave::open(&saves, "Hand Edited World").unwrap();
        let loaded = reopened.load_chunk(-1, 2).unwrap().unwrap();
        assert_eq!(loaded.get_block(15, 77, 3), BlockState::new(300, 7));
        assert_eq!(loaded.get_block(0, 64, 0), BlockState::new(5, 4));
        assert_eq!(loaded.biomes().unwrap()[255], 6);
        assert_eq!(loaded.section(4).unwrap().block_light_at(0, 13, 0), 0x0B);
        assert_eq!(
            loaded.section(4).unwrap().sky_light_at(0, 13, 0),
            Some(0x0F)
        );
        assert_eq!(reopened.metadata().seed, 0xFEDC_BA98_7654_3210);
        assert_eq!(reopened.metadata().mode, WorldMode::Survival);
        assert_eq!(list_worlds(&saves).unwrap().len(), 1);

        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn only_explicitly_saved_chunks_appear_in_the_world() {
        let saves = temp_saves_dir("dirty-only");
        let mut save = WorldSave::create(&saves, "Dirty", 42, WorldMode::Creative).unwrap();
        let mut edited = ChunkColumn::empty(0, 0);
        edited.set_block(1, 70, 2, BlockState::STONE);
        save.save_chunk(&edited).unwrap();

        assert!(save.load_chunk(0, 0).unwrap().is_some());
        assert!(save.load_chunk(1, 0).unwrap().is_none());
        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn rejects_world_names_that_could_escape_the_saves_directory() {
        let saves = temp_saves_dir("invalid-name");
        assert!(matches!(
            WorldSave::create(&saves, "../outside", 1, WorldMode::Creative),
            Err(SaveError::InvalidWorldName(_))
        ));
        assert!(matches!(
            WorldSave::create(&saves, "CON", 1, WorldMode::Creative),
            Err(SaveError::InvalidWorldName(_))
        ));
        let _ = fs::remove_dir_all(saves);
    }
}
