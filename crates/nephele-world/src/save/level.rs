use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

type Fields = IndexMap<String, Tag>;

pub(super) struct LevelDocument {
    pub root_name: String,
    pub root_fields: Fields,
    pub data: Fields,
    pub raw_bytes: Vec<u8>,
}

fn read_document(path: &Path) -> Result<LevelDocument, SaveError> {
    let raw_bytes = fs::read(path)?;
    let mut bytes = Vec::new();
    GzDecoder::new(raw_bytes.as_slice()).read_to_end(&mut bytes)?;
    let (root_name, root) = parse_nbt(&bytes)?;
    let invalid = || SaveError::InvalidLevelData(path.into());
    let Tag::Compound(mut root_fields) = root else {
        return Err(invalid());
    };
    let Some(Tag::Compound(data)) = root_fields.shift_remove("Data") else {
        return Err(invalid());
    };
    metadata_from_level(&data, "")?;
    Ok(LevelDocument {
        root_name,
        root_fields,
        data,
        raw_bytes,
    })
}

pub(super) fn read_level_document(dir: &Path) -> Result<Option<LevelDocument>, SaveError> {
    let mut first_error = None;
    for name in [LEVEL_FILE, "level.dat.bak", "level.dat_old"] {
        match read_document(&dir.join(name)) {
            Ok(doc) => return Ok(Some(doc)),
            Err(SaveError::Io(e)) if e.kind() == io::ErrorKind::NotFound => (),
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
    }
    first_error.map_or(Ok(None), Err)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // rename replaces a regular destination on Windows, without first unlinking it.
    let tmp = sibling(path, "tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

pub(super) fn atomic_write_with_backup(
    path: &Path,
    previous: Option<&[u8]>,
    bytes: &[u8],
) -> io::Result<()> {
    if let Some(previous) = previous {
        replace_file(&sibling(path, "bak"), previous)?;
    }
    replace_file(path, bytes)
}

pub(super) fn metadata_from_level(
    data: &Fields,
    directory: &str,
) -> Result<WorldMetadata, SaveError> {
    let (Some(Tag::String(name)), Some(Tag::Long(seed)), Some(Tag::Int(mode))) = (
        data.get("LevelName"),
        data.get("RandomSeed"),
        data.get("GameType"),
    ) else {
        return Err(SaveError::InvalidLevelData(
            PathBuf::from(directory).join(LEVEL_FILE),
        ));
    };
    Ok(WorldMetadata {
        name: name.clone(),
        directory: directory.into(),
        seed: *seed as u64,
        mode: if *mode == 1 {
            WorldMode::Creative
        } else {
            WorldMode::Survival
        },
    })
}

pub(super) fn new_level_data(meta: &WorldMetadata, world: &WorldState) -> Fields {
    let mut data = Fields::new();
    data.insert("version".into(), Tag::Int(VANILLA_LEVEL_VERSION));
    data.insert("generatorName".into(), Tag::String("default".into()));
    data.insert("generatorVersion".into(), Tag::Int(1));
    data.insert("generatorOptions".into(), Tag::String(String::new()));
    data.insert("MapFeatures".into(), Tag::Byte(1));
    data.insert("initialized".into(), Tag::Byte(1));
    data.insert("hardcore".into(), Tag::Byte(0));
    data.insert(
        "allowCommands".into(),
        Tag::Byte(if meta.mode.is_creative() { 1 } else { 0 }),
    );
    // Normal difficulty is the singleplayer default vanilla falls back to.
    data.insert("Difficulty".into(), Tag::Byte(2));
    data.insert("DifficultyLocked".into(), Tag::Byte(0));
    update_level_fields(&mut data, meta, world);
    data
}

pub(super) fn update_level_fields(data: &mut Fields, meta: &WorldMetadata, w: &WorldState) {
    for (name, value) in [
        ("LevelName", Tag::String(meta.name.clone())),
        ("RandomSeed", Tag::Long(meta.seed as i64)),
        ("GameType", Tag::Int(i32::from(meta.mode.is_creative()))),
        ("Time", Tag::Long(w.total_time)),
        ("DayTime", Tag::Long(w.day_time)),
        ("raining", Tag::Byte(if w.raining { 1 } else { 0 })),
        ("thundering", Tag::Byte(if w.thundering { 1 } else { 0 })),
        ("rainTime", Tag::Int(w.rain_time)),
        ("thunderTime", Tag::Int(w.thunder_time)),
    ] {
        data.insert(name.into(), value);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    data.insert(
        "LastPlayed".into(),
        Tag::Long(now.min(i64::MAX as u128) as i64),
    );
    set_spawn(data, Some(w.spawn_position));
}

fn set_spawn(data: &mut Fields, pos: Option<[i32; 3]>) {
    for (i, key) in ["SpawnX", "SpawnY", "SpawnZ"].into_iter().enumerate() {
        if let Some(pos) = pos {
            data.insert(key.into(), Tag::Int(pos[i]));
        } else {
            data.shift_remove(key);
        }
    }
}

fn spawn(data: &Fields) -> Option<[i32; 3]> {
    Some([
        data.get("SpawnX")?.as_int()?,
        data.get("SpawnY")?.as_int()?,
        data.get("SpawnZ")?.as_int()?,
    ])
}

pub(super) fn world_state_from_level(d: &Fields) -> WorldState {
    let time = d.get("Time").and_then(Tag::as_long).unwrap_or(0);
    WorldState {
        total_time: time,
        day_time: d.get("DayTime").and_then(Tag::as_long).unwrap_or(time),
        spawn_position: spawn(d).unwrap_or([0, 64, 0]),
        raining: matches!(d.get("raining"),Some(Tag::Byte(v)) if *v != 0),
        thundering: matches!(d.get("thundering"),Some(Tag::Byte(v)) if *v != 0),
        rain_time: d.get("rainTime").and_then(Tag::as_int).unwrap_or(6000),
        thunder_time: d.get("thunderTime").and_then(Tag::as_int).unwrap_or(0),
    }
}

pub(super) fn inventory_entries(player: Option<&Tag>) -> &[Tag] {
    player
        .and_then(|p| p.get("Inventory"))
        .and_then(Tag::as_list)
        .unwrap_or(&[])
}

fn container_slot(persisted: i8) -> Option<usize> {
    match persisted {
        0..=8 => Some(persisted as usize + 36),
        9..=35 => Some(persisted as usize),
        100..=103 => Some(108 - persisted as usize),
        _ => None,
    }
}
fn disk_slot(container: usize) -> Option<i8> {
    match container {
        36..=44 => Some((container - 36) as i8),
        9..=35 => Some(container as i8),
        5..=8 => Some((108 - container) as i8),
        _ => None,
    }
}

fn invalid(field: &str) -> SaveError {
    SaveError::InvalidPlayerState(field.into())
}

fn parse_player(player: &Tag, legacy: bool) -> Result<SavedPlayerState, SaveError> {
    let d = player.as_compound().ok_or_else(|| invalid("Player"))?;
    let float = |k, default| match d.get(k) {
        Some(Tag::Float(v)) => *v,
        Some(Tag::Short(v)) => *v as f32,
        _ => default,
    };
    let int = |k, default| d.get(k).and_then(Tag::as_int).unwrap_or(default);
    let (position, yaw, pitch, held_slot) = if legacy {
        let coord = |k| match d.get(k) {
            Some(Tag::Double(v)) => Ok(*v),
            _ => Err(invalid(k)),
        };
        (
            [coord("X")?, coord("Y")?, coord("Z")?],
            float("Yaw", 0.),
            float("Pitch", 0.),
            match d.get("HeldSlot") {
                Some(Tag::Short(v)) => *v,
                _ => 0,
            },
        )
    } else {
        let Some([Tag::Double(x), Tag::Double(y), Tag::Double(z)]) =
            d.get("Pos").and_then(Tag::as_list)
        else {
            return Err(invalid("Pos"));
        };
        let Some([Tag::Float(yaw), Tag::Float(pitch)]) = d.get("Rotation").and_then(Tag::as_list)
        else {
            return Err(invalid("Rotation"));
        };
        (
            [*x, *y, *z],
            (yaw + 180.).to_radians().rem_euclid(std::f32::consts::TAU),
            -pitch.to_radians(),
            int("SelectedItemSlot", 0) as i16,
        )
    };
    let mut inventory = vec![None; PLAYER_INVENTORY_SLOTS];
    for entry in inventory_entries(Some(player)) {
        let Some(Tag::Byte(slot)) = entry.get("Slot") else {
            return Err(invalid("Slot"));
        };
        let idx = if legacy {
            usize::try_from(*slot).ok().filter(|i| *i < 45)
        } else {
            container_slot(*slot)
        };
        let Some(idx) = idx else { continue };
        let id = match entry.get(if legacy { "Id" } else { "id" }) {
            Some(Tag::Short(id)) => *id,
            Some(Tag::String(id)) => named_item(id),
            _ => return Err(invalid("item id")),
        };
        let Some(Tag::Byte(count)) = entry.get("Count") else {
            return Err(invalid("Count"));
        };
        let damage = match entry.get("Damage") {
            Some(Tag::Short(v)) => *v,
            _ => 0,
        };
        let template_slot = (!legacy
            && (matches!(entry.get("id"), Some(Tag::String(_)))
                || entry.as_compound().is_some_and(|d| d.len() > 4)))
        .then_some(*slot as u8);
        inventory[idx] = Some(SavedItemStack {
            id,
            count: *count,
            damage,
            template_slot,
        });
    }
    let state = SavedPlayerState {
        position,
        yaw,
        pitch,
        held_slot,
        inventory,
        health: float("HealF", float("Health", 20.)),
        food_level: int("foodLevel", 20),
        food_saturation: float("foodSaturationLevel", 5.),
        food_exhaustion: float("foodExhaustionLevel", 0.),
        xp_progress: float("XpP", 0.),
        xp_level: int("XpLevel", 0),
        xp_total: int("XpTotal", 0),
        xp_seed: int("XpSeed", 0),
        spawn_point: spawn(d),
    };
    validate_player_state(&state)?;
    Ok(state)
}

// Unrecognised registry names keep their original compound and move as opaque items.
fn named_item(name: &str) -> i16 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    match name {
        "planks" => 5,
        "wool" => 35,
        "log" => 17,
        "grass" => 2,
        "stick" => 280,
        "wooden_pickaxe" => 270,
        "diamond_pickaxe" => 278,
        "diamond" => 264,
        name => crate::blocks::by_name(name).map_or(i16::MAX, |(id, _)| id as i16),
    }
}

pub(super) fn parse_legacy_player(p: &Tag) -> Result<SavedPlayerState, SaveError> {
    parse_player(p, true)
}
pub(super) fn parse_vanilla_player(p: &Tag) -> Result<SavedPlayerState, SaveError> {
    parse_player(p, false)
}

pub(super) fn player_tag_with_state(
    old: Option<&Tag>,
    templates: &[Tag],
    s: &SavedPlayerState,
) -> Fields {
    let mut p = old.and_then(Tag::as_compound).cloned().unwrap_or_default();
    let mut items: Vec<Tag> = inventory_entries(old)
        .iter()
        .filter(
            |e| matches!(e.get("Slot"), Some(Tag::Byte(slot)) if container_slot(*slot).is_none()),
        )
        .cloned()
        .collect();
    for (idx, item) in s.inventory.iter().enumerate() {
        let (Some(slot), Some(item)) = (disk_slot(idx), item) else {
            continue;
        };
        let template = item.template_slot.and_then(|source| {
            templates
                .iter()
                .find(|e| e.get("Slot") == Some(&Tag::Byte(source as i8)))
        });
        let mut stack = template
            .and_then(Tag::as_compound)
            .cloned()
            .unwrap_or_default();
        stack.entry("id".into()).or_insert(Tag::Short(item.id));
        for (key, value) in [
            ("Slot", Tag::Byte(slot)),
            ("Count", Tag::Byte(item.count)),
            ("Damage", Tag::Short(item.damage)),
        ] {
            stack.insert(key.into(), value);
        }
        items.push(Tag::Compound(stack));
    }
    for (k, v) in [
        ("Pos", Tag::List(s.position.map(Tag::Double).to_vec())),
        (
            "Rotation",
            Tag::List(vec![
                Tag::Float(s.yaw.to_degrees() - 180.),
                Tag::Float(-s.pitch.to_degrees()),
            ]),
        ),
        ("HealF", Tag::Float(s.health)),
        ("Health", Tag::Short(s.health.ceil() as i16)),
        ("foodLevel", Tag::Int(s.food_level)),
        ("foodSaturationLevel", Tag::Float(s.food_saturation)),
        ("foodExhaustionLevel", Tag::Float(s.food_exhaustion)),
        ("XpP", Tag::Float(s.xp_progress)),
        ("XpLevel", Tag::Int(s.xp_level)),
        ("XpTotal", Tag::Int(s.xp_total)),
        ("XpSeed", Tag::Int(s.xp_seed)),
        ("SelectedItemSlot", Tag::Int(i32::from(s.held_slot))),
        ("Inventory", Tag::List(items)),
    ] {
        p.insert(k.into(), v);
    }
    p.entry("Motion".into())
        .or_insert_with(|| Tag::List(vec![Tag::Double(0.); 3]));
    p.entry("Air".into()).or_insert(Tag::Short(300));
    set_spawn(&mut p, s.spawn_point);
    p
}
