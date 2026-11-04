use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

// Rejected by validate_directory_name, so neither snapshots nor partial copies can be played.
const ARCHIVE: &str = "..nephele-backups";

pub(super) fn plain_directory(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a directory, not a link",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "linked directory in save",
            ));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    plain_directory(source)?;
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if ty.is_file() {
            fs::copy(entry.path(), target)?;
        } else if ty.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "linked or special file in save",
            ));
        }
    }
    Ok(())
}

fn unique_copy_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn read_backup(archive: &Path, id: &str) -> Result<WorldSave, SaveError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SaveError::InvalidWorldName(id.to_owned()));
    }
    plain_directory(archive)?;
    let snapshot = archive.join(id);
    plain_directory(&snapshot)?;
    let mut entries = fs::read_dir(&snapshot)?;
    let entry = entries
        .next()
        .transpose()?
        .ok_or_else(|| SaveError::InvalidMetadata(snapshot.clone()))?;
    if entries.next().is_some() {
        return Err(SaveError::InvalidMetadata(snapshot));
    }
    plain_directory(&entry.path())?;
    WorldSave::open(snapshot, &entry.file_name().to_string_lossy())
}

impl WorldSave {
    pub fn backup(saves_dir: impl AsRef<Path>, directory: &str) -> Result<PathBuf, SaveError> {
        let saves = saves_dir.as_ref();
        let world = Self::open(saves, directory)?;
        let archive = saves.join(ARCHIVE);
        fs::create_dir_all(&archive)?;
        plain_directory(&archive)?;
        let id = unique_copy_id();
        let stage = archive.join(format!("{id}.tmp"));
        let destination = archive.join(id);
        fs::create_dir(&stage)?;
        let result = copy_tree(&world.path, &stage.join(directory))
            .and_then(|()| fs::rename(&stage, &destination));
        if let Err(error) = result {
            let _ = fs::remove_dir_all(stage);
            return Err(error.into());
        }
        Ok(destination.join(directory))
    }

    /// Latest first. The directory field is the immutable snapshot identifier.
    pub fn list_backups(saves_dir: impl AsRef<Path>) -> Result<Vec<WorldMetadata>, SaveError> {
        let archive = saves_dir.as_ref().join(ARCHIVE);
        match plain_directory(&archive) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            result => result?,
        }
        let mut backups = Vec::new();
        for entry in fs::read_dir(&archive)? {
            let id = entry?.file_name().to_string_lossy().into_owned();
            if let Ok(world) = read_backup(&archive, &id) {
                backups.push(WorldMetadata {
                    directory: id,
                    ..world.metadata
                });
            }
        }
        backups.sort_by(|a, b| {
            b.directory
                .len()
                .cmp(&a.directory.len())
                .then_with(|| b.directory.cmp(&a.directory))
        });
        Ok(backups)
    }

    /// Restores an independent playable copy. Even if the original still exists, never replace it.
    pub fn restore_backup(
        saves_dir: impl AsRef<Path>,
        id: &str,
    ) -> Result<WorldMetadata, SaveError> {
        let saves = saves_dir.as_ref();
        let source = read_backup(&saves.join(ARCHIVE), id)?;
        let base = &source.metadata.directory;
        let mut name = base.clone();
        let mut n = 0usize;
        while saves.join(&name).try_exists()? {
            n += 1;
            let suffix = if n == 1 {
                " (restored)".to_string()
            } else {
                format!(" (restored {n})")
            };
            let mut prefix = base.as_str();
            while prefix.encode_utf16().count() + suffix.len() > MAX_WORLD_NAME_UNITS {
                prefix = &prefix[..prefix.char_indices().next_back().unwrap().0];
            }
            name = format!("{prefix}{suffix}");
        }
        let stage = saves.join(format!("..restore-{}", unique_copy_id()));
        fs::create_dir(&stage)?;
        let result = (|| {
            copy_tree(&source.path, &stage.join(base))?;
            let mut world = Self::open(&stage, base)?;
            if name != *base {
                world.rename(&name)?;
            }
            fs::rename(&world.path, saves.join(name))?;
            Ok(world.metadata)
        })();
        let _ = fs::remove_dir_all(stage);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockState, ChunkColumn};

    #[test]
    fn world_list_lifecycle_keeps_original_neighbour_and_snapshot_independent() {
        let saves = super::super::tests::temp_saves_dir("list-lifecycle");
        let mut a = WorldSave::create(&saves, "Selected", 123, WorldMode::Survival).unwrap();
        let neighbour = WorldSave::create(&saves, "Other", 456, WorldMode::Creative).unwrap();
        let untouched = fs::read(neighbour.path.join(LEVEL_FILE)).unwrap();
        let mut chunk = ChunkColumn::empty(-1, 0);
        chunk.set_block(15, 72, 2, BlockState::new(35, 14));
        a.save_chunk(&chunk).unwrap();
        let backup = WorldSave::backup(&saves, "Selected").unwrap();
        assert!(backup.join("region/r.-1.0.mca").is_file());
        let id = &WorldSave::list_backups(&saves).unwrap()[0].directory;
        assert_eq!(list_worlds(&saves).unwrap().len(), 2);
        let restored = WorldSave::restore_backup(&saves, id).unwrap();
        assert_eq!(restored.directory, "Selected (restored)");
        a.rename("Renamed").unwrap();
        assert!(a.rename("Other").is_err());
        WorldSave::delete(&saves, "Renamed").unwrap();
        assert_eq!(
            WorldSave::restore_backup(&saves, id).unwrap().directory,
            "Selected"
        );
        assert!(WorldSave::restore_backup(&saves, "../Other").is_err());
        for name in ["Selected", "Selected (restored)"] {
            let mut w = WorldSave::open(&saves, name).unwrap();
            assert_eq!(
                w.load_chunk(-1, 0).unwrap().unwrap().get_block(15, 72, 2),
                BlockState::new(35, 14)
            );
        }
        assert_eq!(
            fs::read(neighbour.path.join(LEVEL_FILE)).unwrap(),
            untouched
        );
        fs::remove_dir_all(saves).unwrap();
    }
}
