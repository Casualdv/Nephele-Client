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
const PLAYER_STATE_FILE: &str = "player.dat";
const REGION_DIR: &str = "region";
const MAX_WORLD_NAME_UNITS: usize = 64;
const PLAYER_INVENTORY_SLOTS: usize = 45;

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedPlayerState {
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub held_slot: i16,
    pub inventory: Vec<Option<SavedItemStack>>,
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

    /// Loads the saved local player's position, orientation, selected slot and inventory.
    pub fn load_player_state(&self) -> Result<Option<SavedPlayerState>, SaveError> {
        let path = self.path.join(PLAYER_STATE_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let (_, root) = parse_nbt(&bytes)?;
        let read_double = |name: &str| match root.get(name) {
            Some(Tag::Double(value)) => Ok(*value),
            _ => Err(SaveError::InvalidPlayerState(format!("missing {name}"))),
        };
        let yaw = match root.get("Yaw") {
            Some(Tag::Float(value)) => *value,
            _ => return Err(SaveError::InvalidPlayerState("missing yaw".to_string())),
        };
        let pitch = match root.get("Pitch") {
            Some(Tag::Float(value)) => *value,
            _ => return Err(SaveError::InvalidPlayerState("missing pitch".to_string())),
        };
        let held_slot = match root.get("HeldSlot") {
            Some(Tag::Short(value)) => *value,
            _ => {
                return Err(SaveError::InvalidPlayerState(
                    "missing held slot".to_string(),
                ));
            }
        };
        let entries = root
            .get("Inventory")
            .and_then(Tag::as_list)
            .ok_or_else(|| SaveError::InvalidPlayerState("missing inventory".to_string()))?;
        let mut inventory = vec![None; PLAYER_INVENTORY_SLOTS];
        for entry in entries {
            let slot = match entry.get("Slot") {
                Some(Tag::Byte(value)) if (0..PLAYER_INVENTORY_SLOTS as i8).contains(value) => {
                    *value as usize
                }
                _ => continue,
            };
            let (Some(Tag::Short(id)), Some(Tag::Byte(count)), Some(Tag::Short(damage))) =
                (entry.get("Id"), entry.get("Count"), entry.get("Damage"))
            else {
                continue;
            };
            if *count > 0 {
                inventory[slot] = Some(SavedItemStack {
                    id: *id,
                    count: *count,
                    damage: *damage,
                });
            }
        }

        let state = SavedPlayerState {
            position: [read_double("X")?, read_double("Y")?, read_double("Z")?],
            yaw,
            pitch,
            held_slot,
            inventory,
        };
        validate_player_state(&state)?;
        Ok(Some(state))
    }

    /// Writes the local player's state into this world's `player.dat`.
    pub fn save_player_state(&self, state: &SavedPlayerState) -> Result<(), SaveError> {
        validate_player_state(state)?;

        let mut inventory = Vec::new();
        for (slot, item) in state.inventory.iter().enumerate() {
            let Some(item) = item else {
                continue;
            };
            let mut stack = HashMap::new();
            stack.insert("Slot".to_string(), Tag::Byte(slot as i8));
            stack.insert("Id".to_string(), Tag::Short(item.id));
            stack.insert("Count".to_string(), Tag::Byte(item.count));
            stack.insert("Damage".to_string(), Tag::Short(item.damage));
            inventory.push(Tag::Compound(stack));
        }

        let mut player = HashMap::new();
        player.insert("Version".to_string(), Tag::Int(1));
        player.insert("X".to_string(), Tag::Double(state.position[0]));
        player.insert("Y".to_string(), Tag::Double(state.position[1]));
        player.insert("Z".to_string(), Tag::Double(state.position[2]));
        player.insert("Yaw".to_string(), Tag::Float(state.yaw));
        player.insert("Pitch".to_string(), Tag::Float(state.pitch));
        player.insert("HeldSlot".to_string(), Tag::Short(state.held_slot));
        player.insert("Inventory".to_string(), Tag::List(inventory));

        let bytes = encode_nbt("Player", &Tag::Compound(player))?;
        fs::write(self.path.join(PLAYER_STATE_FILE), bytes)?;
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

        if same_path {
            self.metadata.name = name.to_string();
            if let Err(error) = write_metadata(&self.path.join(METADATA_FILE), &self.metadata) {
                self.metadata = previous;
                return Err(error);
            }
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
        self.regions.clear();
        if let Err(error) = write_metadata(&self.path.join(METADATA_FILE), &self.metadata) {
            self.path = old_path.clone();
            self.metadata = previous;
            self.regions.clear();
            let _ = fs::rename(new_path, old_path);
            return Err(error);
        }
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
    fn player_position_orientation_and_inventory_roundtrip_in_world_data() {
        let saves = temp_saves_dir("player-state");
        let save = WorldSave::create(&saves, "Player State", 91, WorldMode::Survival).unwrap();
        let mut inventory = vec![None; PLAYER_INVENTORY_SLOTS];
        inventory[0] = Some(SavedItemStack {
            id: 4,
            count: 31,
            damage: 2,
        });
        inventory[44] = Some(SavedItemStack {
            id: 280,
            count: 5,
            damage: 0,
        });
        let player = SavedPlayerState {
            position: [-23.5, 72.25, 41.75],
            yaw: 1.25,
            pitch: -0.375,
            held_slot: 7,
            inventory,
        };

        save.save_player_state(&player).unwrap();
        drop(save);

        let reopened = WorldSave::open(&saves, "Player State").unwrap();
        assert_eq!(reopened.load_player_state().unwrap(), Some(player));
        assert!(saves.join("Player State").join(PLAYER_STATE_FILE).is_file());

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
}
