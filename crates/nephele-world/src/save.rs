//! Single-player world metadata and Anvil chunk persistence.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indexmap::IndexMap;
use nephele_anvil::{
    encode_chunk_nbt, encode_nbt, parse_chunk, parse_nbt, RegionFile, StoredChunk, StoredSection,
    Tag,
};

use crate::chunk::{ChunkColumn, Section, SECTION_COUNT, SECTION_VOLUME};

const METADATA_FILE: &str = "nephele.dat";
const PLAYER_STATE_FILE: &str = "player.dat";
const LEVEL_FILE: &str = "level.dat";
const REGION_DIR: &str = "region";
mod level;
mod management;
use level::*;
const MAX_WORLD_NAME_UNITS: usize = 64;
const PLAYER_INVENTORY_SLOTS: usize = 45;
const VANILLA_LEVEL_VERSION: i32 = 19133;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedItemStack {
    pub id: i16,
    pub count: i8,
    pub damage: i16,
    pub template_slot: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedPlayerState {
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub food_level: i32,
    pub food_saturation: f32,
    pub food_exhaustion: f32,
    pub xp_progress: f32,
    pub xp_level: i32,
    pub xp_total: i32,
    pub xp_seed: i32,
    pub held_slot: i16,
    pub spawn_point: Option<[i32; 3]>,
    pub inventory: Vec<Option<SavedItemStack>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldState {
    pub total_time: i64,
    pub day_time: i64,
    pub spawn_position: [i32; 3],
    pub raining: bool,
    pub rain_time: i32,
    pub thundering: bool,
    pub thunder_time: i32,
}

impl Default for WorldState {
    fn default() -> Self {
        Self {
            total_time: 0,
            day_time: 0,
            spawn_position: [0, 64, 0],
            raining: false,
            rain_time: 6_000,
            thundering: false,
            thunder_time: 0,
        }
    }
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
    InvalidLevelData(PathBuf),
    InvalidPlayerState(String),
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
            Self::InvalidLevelData(path) => {
                write!(
                    f,
                    "invalid or corrupt vanilla level data at {}",
                    path.display()
                )
            }
            Self::InvalidPlayerState(reason) => write!(f, "invalid saved player state: {reason}"),
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
            | Self::InvalidLevelData(_)
            | Self::InvalidPlayerState(_)
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
    level_root_name: String,
    root_fields: IndexMap<String, Tag>,
    source_inventory: Vec<Tag>,
    level_data: IndexMap<String, Tag>,
    last_good_level_bytes: Option<Vec<u8>>,
}

impl WorldSave {
    /// Creates a vanilla-readable save at `<saves_dir>/<name>`.
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
        let level_data = new_level_data(&metadata, &WorldState::default());
        let mut save = Self {
            path,
            metadata,
            regions: HashMap::new(),
            level_root_name: String::new(),
            root_fields: IndexMap::new(),
            source_inventory: Vec::new(),
            level_data,
            last_good_level_bytes: None,
        };
        if let Err(error) = save
            .write_level_dat()
            .and_then(|()| write_legacy_metadata(&save.path, &save.metadata))
        {
            let _ = fs::remove_dir_all(&save.path);
            return Err(error);
        }
        Ok(save)
    }

    /// Opens an existing save by its directory name.
    pub fn open(saves_dir: impl AsRef<Path>, directory: &str) -> Result<Self, SaveError> {
        let directory = validate_directory_name(directory)?;
        let path = saves_dir.as_ref().join(&directory);
        management::plain_directory(&path)?;
        let document = read_level_document(&path)?;
        let (level_root_name, root_fields, level_data, last_good_level_bytes, metadata) =
            match document {
                Some(document) => {
                    let metadata = metadata_from_level(&document.data, &directory)
                        .or_else(|_| read_metadata(&path.join(METADATA_FILE)))?;
                    (
                        document.root_name,
                        document.root_fields,
                        document.data,
                        Some(document.raw_bytes),
                        metadata,
                    )
                }
                None => {
                    let metadata = read_metadata(&path.join(METADATA_FILE))?;
                    let level_data = new_level_data(&metadata, &WorldState::default());
                    (String::new(), IndexMap::new(), level_data, None, metadata)
                }
            };
        Ok(Self {
            path,
            metadata: WorldMetadata {
                directory,
                ..metadata
            },
            regions: HashMap::new(),
            level_root_name,
            root_fields,
            source_inventory: inventory_entries(level_data.get("Player")).to_vec(),
            level_data,
            last_good_level_bytes,
        })
    }

    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    pub fn world_state(&self) -> WorldState {
        world_state_from_level(&self.level_data)
    }

    /// Persists a changed mode in the world's metadata.
    pub fn set_mode(&mut self, mode: WorldMode) -> Result<(), SaveError> {
        if self.metadata.mode == mode {
            return Ok(());
        }
        let previous = self.metadata.mode;
        self.metadata.mode = mode;
        self.level_data.insert(
            "GameType".to_string(),
            Tag::Int(match mode {
                WorldMode::Survival => 0,
                WorldMode::Creative => 1,
            }),
        );
        self.level_data.insert(
            "LevelName".to_string(),
            Tag::String(self.metadata.name.clone()),
        );
        if let Err(error) = self.write_level_dat() {
            self.metadata.mode = previous;
            self.level_data.insert(
                "GameType".to_string(),
                Tag::Int(if previous.is_creative() { 1 } else { 0 }),
            );
            return Err(error);
        }
        let _ = write_legacy_metadata(&self.path, &self.metadata);
        Ok(())
    }

    /// Loads the vanilla `Data.Player` compound, falling back to Nephele's
    /// pre-level.dat player file for worlds created by early M5 builds.
    pub fn load_player_state(&self) -> Result<Option<SavedPlayerState>, SaveError> {
        if let Some(player) = self.level_data.get("Player") {
            return parse_vanilla_player(player).map(Some);
        }
        let legacy = self.path.join(PLAYER_STATE_FILE);
        if legacy.is_file() {
            let bytes = fs::read(&legacy)?;
            let (_, root) = parse_nbt(&bytes)?;
            return parse_legacy_player(&root).map(Some);
        }
        Ok(None)
    }

    /// Writes player state under vanilla's `Data.Player` in `level.dat`.
    pub fn save_player_state(&mut self, state: &SavedPlayerState) -> Result<(), SaveError> {
        validate_player_state(state)?;
        let player =
            player_tag_with_state(self.level_data.get("Player"), &self.source_inventory, state);
        self.level_data
            .insert("Player".to_string(), Tag::Compound(player));
        self.write_level_dat()
    }

    /// Atomically persists level/player state together, keeping the previous
    /// complete `level.dat` in `level.dat.bak` before replacing it.
    pub fn save_snapshot(
        &mut self,
        player: &SavedPlayerState,
        world: &WorldState,
    ) -> Result<(), SaveError> {
        validate_player_state(player)?;
        update_level_fields(&mut self.level_data, &self.metadata, world);
        let updated_player = player_tag_with_state(
            self.level_data.get("Player"),
            &self.source_inventory,
            player,
        );
        self.level_data
            .insert("Player".to_string(), Tag::Compound(updated_player));
        self.write_level_dat()
    }

    pub fn save_world_state(&mut self, world: &WorldState) -> Result<(), SaveError> {
        update_level_fields(&mut self.level_data, &self.metadata, world);
        self.write_level_dat()
    }

    fn write_level_dat(&mut self) -> Result<(), SaveError> {
        let mut root = self.root_fields.clone();
        root.insert("Data".into(), Tag::Compound(self.level_data.clone()));
        let raw = encode_nbt(&self.level_root_name, &Tag::Compound(root))?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        let bytes = encoder.finish()?;
        atomic_write_with_backup(
            &self.path.join(LEVEL_FILE),
            self.last_good_level_bytes.as_deref(),
            &bytes,
        )?;
        self.last_good_level_bytes = Some(bytes);
        Ok(())
    }

    /// Renames the world folder and display name after validating the new name.
    pub fn rename(&mut self, new_name: &str) -> Result<WorldMetadata, SaveError> {
        let name = new_name.trim();
        let directory = validate_directory_name(name)?;
        let same_path = if cfg!(windows) {
            self.metadata.directory.eq_ignore_ascii_case(&directory)
        } else {
            self.metadata.directory == directory
        };
        let previous = self.metadata.clone();
        let previous_level_data = self.level_data.clone();

        if same_path {
            self.metadata.name = name.to_string();
            self.level_data.insert(
                "LevelName".to_string(),
                Tag::String(self.metadata.name.clone()),
            );
            if let Err(error) = self.write_level_dat() {
                self.metadata = previous;
                self.level_data = previous_level_data;
                return Err(error);
            }
            let _ = write_legacy_metadata(&self.path, &self.metadata);
            return Ok(self.metadata.clone());
        }

        let new_path = self.path.with_file_name(&directory);
        if new_path.exists() {
            return Err(SaveError::AlreadyExists(new_path));
        }
        let old_path = self.path.clone();
        fs::rename(&old_path, &new_path)?;
        self.path = new_path.clone();
        self.metadata.directory = directory;
        self.metadata.name = name.to_string();
        self.level_data.insert(
            "LevelName".to_string(),
            Tag::String(self.metadata.name.clone()),
        );
        self.regions.clear();
        if let Err(error) = self.write_level_dat() {
            self.path = old_path.clone();
            self.metadata = previous;
            self.level_data = previous_level_data;
            self.regions.clear();
            let _ = fs::rename(new_path, old_path);
            return Err(error);
        }
        let _ = write_legacy_metadata(&self.path, &self.metadata);
        Ok(self.metadata.clone())
    }

    /// Deletes an existing world after requiring a valid world metadata file.
    pub fn delete(saves_dir: impl AsRef<Path>, directory: &str) -> Result<(), SaveError> {
        let world = Self::open(saves_dir, directory)?;
        fs::remove_dir_all(world.path)?;
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
        self.save_chunks(std::slice::from_ref(column))
    }

    /// Writes a batch of dirty columns, compacting each touched region once.
    pub fn save_chunks(&mut self, columns: &[ChunkColumn]) -> Result<(), SaveError> {
        type RegionUpdates = Vec<(usize, usize, Vec<u8>)>;
        let mut by_region: BTreeMap<(i32, i32), RegionUpdates> = BTreeMap::new();
        for column in columns {
            let region_coord = (column.x.div_euclid(32), column.z.div_euclid(32));
            let nbt = encode_chunk_nbt(&stored_from_column(column))?;
            by_region.entry(region_coord).or_default().push((
                column.x.rem_euclid(32) as usize,
                column.z.rem_euclid(32) as usize,
                nbt,
            ));
        }
        for (region_coord, updates) in by_region {
            RegionFile::write_chunks(self.region_path(region_coord), &updates)?;
            self.regions.remove(&region_coord);
        }
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
        if let Ok(world) = WorldSave::open(saves_dir, &directory) {
            worlds.push(world.metadata);
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
        || name.encode_utf16().count() > MAX_WORLD_NAME_UNITS
        || name != name.trim()
        || name == "."
        || name.contains("..")
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        });
    let device_name = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (1..=9).any(|index| {
            device_name == format!("COM{index}") || device_name == format!("LPT{index}")
        })
        || matches!(
            device_name.as_str(),
            "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
        );
    if invalid || reserved {
        return Err(SaveError::InvalidWorldName(name.to_string()));
    }
    Ok(name.to_string())
}

fn validate_player_state(state: &SavedPlayerState) -> Result<(), SaveError> {
    if state.inventory.len() != PLAYER_INVENTORY_SLOTS {
        return Err(SaveError::InvalidPlayerState(format!(
            "inventory has {} slots, expected {PLAYER_INVENTORY_SLOTS}",
            state.inventory.len()
        )));
    }
    if state.position.iter().any(|value| !value.is_finite()) {
        return Err(SaveError::InvalidPlayerState(
            "position must contain finite coordinates".to_string(),
        ));
    }
    if !state.yaw.is_finite() || !state.pitch.is_finite() {
        return Err(SaveError::InvalidPlayerState(
            "yaw and pitch must be finite".to_string(),
        ));
    }
    if !(0..=8).contains(&state.held_slot) {
        return Err(SaveError::InvalidPlayerState(
            "held slot must be in 0..=8".to_string(),
        ));
    }
    if state.inventory.iter().flatten().any(|item| item.count <= 0) {
        return Err(SaveError::InvalidPlayerState(
            "item counts must be positive".to_string(),
        ));
    }
    Ok(())
}

fn write_legacy_metadata(directory: &Path, metadata: &WorldMetadata) -> Result<(), SaveError> {
    let path = directory.join(METADATA_FILE);
    let mut values = IndexMap::new();
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

    pub(super) fn temp_saves_dir(label: &str) -> PathBuf {
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
    fn player_position_orientation_and_inventory_roundtrip_in_world_data() {
        let saves = temp_saves_dir("player-state");
        let mut save = WorldSave::create(&saves, "Player State", 91, WorldMode::Survival).unwrap();
        let mut inventory = vec![None; PLAYER_INVENTORY_SLOTS];
        inventory[36] = Some(SavedItemStack {
            id: 4,
            count: 31,
            damage: 2,
            template_slot: None,
        });
        inventory[44] = Some(SavedItemStack {
            id: 280,
            count: 5,
            damage: 0,
            template_slot: None,
        });
        let player = SavedPlayerState {
            position: [-23.5, 72.25, 41.75],
            health: 12.5,
            food_level: 7,
            food_saturation: 2.5,
            food_exhaustion: 1.75,
            xp_progress: 0.625,
            xp_level: 18,
            xp_total: 415,
            xp_seed: -71,
            spawn_point: Some([-10, 72, 19]),
            yaw: 1.25,
            pitch: -0.375,
            held_slot: 7,
            inventory,
        };

        save.save_player_state(&player).unwrap();
        drop(save);

        let reopened = WorldSave::open(&saves, "Player State").unwrap();
        assert_eq!(reopened.load_player_state().unwrap(), Some(player));
        assert!(saves.join("Player State").join(LEVEL_FILE).is_file());

        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn rename_uses_world_name_validation_and_keeps_saved_chunks() {
        let saves = temp_saves_dir("rename");
        let mut save = WorldSave::create(&saves, "Before", 17, WorldMode::Creative).unwrap();
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(2, 65, 3, BlockState::STONE);
        save.save_chunk(&column).unwrap();

        let too_long = "x".repeat(MAX_WORLD_NAME_UNITS + 1);
        for invalid in ["..", "../outside", "a\\b", "NUL", "COM1", too_long.as_str()] {
            assert!(matches!(
                save.rename(invalid),
                Err(SaveError::InvalidWorldName(_))
            ));
        }
        assert_eq!(save.metadata().directory, "Before");
        save.rename("After").unwrap();

        assert!(!saves.join("Before").exists());
        assert!(saves.join("After").join("region/r.0.0.mca").is_file());
        let mut reopened = WorldSave::open(&saves, "After").unwrap();
        assert_eq!(reopened.metadata().name, "After");
        assert_eq!(
            reopened
                .load_chunk(0, 0)
                .unwrap()
                .unwrap()
                .get_block(2, 65, 3),
            BlockState::STONE
        );

        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn deleting_a_world_removes_its_own_data() {
        let saves = temp_saves_dir("delete");
        let save = WorldSave::create(&saves, "Remove Me", 23, WorldMode::Survival).unwrap();
        let world_path = saves.join("Remove Me");
        drop(save);

        WorldSave::delete(&saves, "Remove Me").unwrap();

        assert!(!world_path.exists());
        assert!(list_worlds(&saves).unwrap().is_empty());
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
        let too_long = "x".repeat(MAX_WORLD_NAME_UNITS + 1);
        for name in [
            "",
            "   ",
            ".",
            "..",
            "../outside",
            "..\\..\\evil",
            "nested/name",
            "nested\\name",
            "world..backup",
            "CON",
            "con.txt",
            "NUL",
            "PRN.log",
            "AUX",
            "COM1",
            "com9.dat",
            "LPT1",
            "lpt9.backup",
            "COM¹",
            "LPT³",
            too_long.as_str(),
        ] {
            assert!(
                matches!(
                    WorldSave::create(&saves, name, 1, WorldMode::Creative),
                    Err(SaveError::InvalidWorldName(_))
                ),
                "accepted unsafe world name {name:?}"
            );
        }
        assert_eq!(fs::read_dir(&saves).unwrap().count(), 0);
        let _ = fs::remove_dir_all(saves);
    }

    fn sample_player() -> SavedPlayerState {
        let mut inventory = vec![None; PLAYER_INVENTORY_SLOTS];
        inventory[36] = Some(SavedItemStack {
            id: 276,
            count: 1,
            damage: 12,
            template_slot: None,
        });
        inventory[9] = Some(SavedItemStack {
            id: 4,
            count: 64,
            damage: 0,
            template_slot: None,
        });
        SavedPlayerState {
            position: [12.5, 71.0, -40.25],
            yaw: 1.25,
            pitch: -0.4,
            health: 17.0,
            food_level: 15,
            food_saturation: 3.5,
            food_exhaustion: 0.25,
            xp_progress: 0.5,
            xp_level: 4,
            xp_total: 55,
            xp_seed: 123,
            held_slot: 3,
            spawn_point: Some([10, 71, -38]),
            inventory,
        }
    }

    fn read_level_fields(path: &Path) -> (String, IndexMap<String, Tag>, IndexMap<String, Tag>) {
        let raw = fs::read(path).unwrap();
        let mut plain = Vec::new();
        GzDecoder::new(raw.as_slice())
            .read_to_end(&mut plain)
            .unwrap();
        let (name, tag) = parse_nbt(&plain).unwrap();
        let Tag::Compound(mut root) = tag else {
            panic!("level.dat root is not a compound");
        };
        let Some(Tag::Compound(data)) = root.shift_remove("Data") else {
            panic!("level.dat has no Data compound");
        };
        (name, root, data)
    }

    fn write_level_fields(
        path: &Path,
        name: &str,
        root: &IndexMap<String, Tag>,
        data: &IndexMap<String, Tag>,
    ) {
        let mut root = root.clone();
        root.insert("Data".into(), Tag::Compound(data.clone()));
        let raw = encode_nbt(name, &Tag::Compound(root)).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw).unwrap();
        fs::write(path, encoder.finish().unwrap()).unwrap();
    }

    #[test]
    fn world_time_weather_and_spawn_roundtrip_through_level_dat() {
        let saves = temp_saves_dir("world-state");
        let mut save = WorldSave::create(&saves, "Weather", 99, WorldMode::Survival).unwrap();
        let state = WorldState {
            total_time: 48_123,
            day_time: 17_400,
            spawn_position: [3, 68, -11],
            raining: true,
            rain_time: 4_321,
            thundering: true,
            thunder_time: 654,
        };
        save.save_snapshot(&sample_player(), &state).unwrap();
        drop(save);

        let reopened = WorldSave::open(&saves, "Weather").unwrap();
        assert_eq!(reopened.world_state(), state);
        assert_eq!(reopened.load_player_state().unwrap(), Some(sample_player()));
        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn level_dat_roundtrip_keeps_unknown_tags_identical() {
        let saves = temp_saves_dir("unknown-tags");
        let save = WorldSave::create(&saves, "Unknown", 77, WorldMode::Creative).unwrap();
        let path = saves.join("Unknown").join(LEVEL_FILE);
        drop(save);

        let (root_name, root, mut data) = read_level_fields(&path);
        let mut custom = IndexMap::new();
        custom.insert("flag".into(), Tag::Byte(-3));
        custom.insert("text".into(), Tag::String("mod data".into()));
        let mut deep = IndexMap::new();
        deep.insert("float".into(), Tag::Float(0.5));
        deep.insert(
            "list".into(),
            Tag::List(vec![Tag::Short(7), Tag::Short(-8)]),
        );
        custom.insert("deep".into(), Tag::Compound(deep));
        data.insert("ModCompound".into(), Tag::Compound(custom.clone()));
        data.insert("ModEmptyList".into(), Tag::EmptyList(10));
        data.insert("ModLongs".into(), Tag::LongArray(vec![5, -6, 7]));
        data.insert("ModInts".into(), Tag::IntArray(vec![-1, 2]));
        write_level_fields(&path, &root_name, &root, &data);

        let mut save = WorldSave::open(&saves, "Unknown").unwrap();
        let state = WorldState {
            total_time: 2_400,
            day_time: 1_200,
            spawn_position: [4, 70, -9],
            raining: true,
            rain_time: 10,
            thundering: false,
            thunder_time: 0,
        };
        save.save_snapshot(&sample_player(), &state).unwrap();

        let (name, _, after) = read_level_fields(&path);
        assert_eq!(name, root_name);
        assert_eq!(after.get("ModCompound"), Some(&Tag::Compound(custom)));
        assert_eq!(after.get("ModEmptyList"), Some(&Tag::EmptyList(10)));
        assert_eq!(after.get("ModLongs"), Some(&Tag::LongArray(vec![5, -6, 7])));
        assert_eq!(after.get("ModInts"), Some(&Tag::IntArray(vec![-1, 2])));
        // Known vanilla tags are refreshed alongside the preserved ones.
        assert_eq!(after.get("LevelName"), Some(&Tag::String("Unknown".into())));
        assert_eq!(after.get("DayTime"), Some(&Tag::Long(1_200)));
        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn player_save_keeps_unknown_player_fields() {
        let saves = temp_saves_dir("player-unknown");
        let mut save = WorldSave::create(&saves, "Player Mods", 31, WorldMode::Survival).unwrap();
        save.save_player_state(&sample_player()).unwrap();
        let path = saves.join("Player Mods").join(LEVEL_FILE);
        drop(save);

        let (root_name, root, mut data) = read_level_fields(&path);
        let mut player = data
            .get("Player")
            .and_then(Tag::as_compound)
            .cloned()
            .expect("saved player tag");
        player.insert("NeonAbility".into(), Tag::Int(9));
        let mut custom = IndexMap::new();
        custom.insert("charge".into(), Tag::Double(1.5));
        player.insert("NeonState".into(), Tag::Compound(custom.clone()));
        data.insert("Player".into(), Tag::Compound(player));
        write_level_fields(&path, &root_name, &root, &data);

        let mut save = WorldSave::open(&saves, "Player Mods").unwrap();
        let mut changed = sample_player();
        changed.health = 9.5;
        save.save_player_state(&changed).unwrap();
        drop(save);

        let (_, _, after) = read_level_fields(&path);
        let player = after
            .get("Player")
            .and_then(Tag::as_compound)
            .expect("player survives");
        assert_eq!(player.get("NeonAbility"), Some(&Tag::Int(9)));
        assert_eq!(player.get("NeonState"), Some(&Tag::Compound(custom)));
        assert_eq!(player.get("HealF"), Some(&Tag::Float(9.5)));
        assert_eq!(
            WorldSave::open(&saves, "Player Mods")
                .unwrap()
                .load_player_state()
                .unwrap()
                .unwrap()
                .health,
            9.5
        );
        let _ = fs::remove_dir_all(saves);
    }

    #[test]
    fn saving_keeps_the_previous_level_dat_and_recovers_from_a_truncated_one() {
        let saves = temp_saves_dir("level-backup");
        let mut save = WorldSave::create(&saves, "Backup", 5, WorldMode::Survival).unwrap();
        let level = saves.join("Backup").join(LEVEL_FILE);
        let backup = saves.join("Backup").join("level.dat.bak");

        let mut state = WorldState {
            total_time: 100,
            ..WorldState::default()
        };
        save.save_world_state(&state).unwrap();
        let first = fs::read(&level).unwrap();

        state.total_time = 200;
        save.save_world_state(&state).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), first);
        assert_ne!(fs::read(&level).unwrap(), first);

        // A process dying mid-write must leave the backup loadable.
        fs::write(&level, b"truncated gzip").unwrap();
        let reopened = WorldSave::open(&saves, "Backup").unwrap();
        assert_eq!(reopened.world_state().total_time, 100);
        let _ = fs::remove_dir_all(saves);
    }
}
