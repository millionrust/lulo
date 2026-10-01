//! Versioned sidebar favourites shared by Files and the Open/Save chooser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

pub const MAX_FAVOURITES: usize = 32;
const MAX_FILE_BYTES: usize = 16 * 1024;
const VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct SavedFavourites {
    version: u32,
    paths: Vec<PathBuf>,
}

fn state_path() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local/state"))
        })?;
    Some(state_home.join("rmac/files/favourites.json"))
}

/// Keep missing targets: Files must show them until the user chooses to remove
/// them. Only absolute, distinct paths are accepted; files are valid too.
pub fn sanitise(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| path.is_absolute() && seen.insert(path.clone()))
        .take(MAX_FAVOURITES)
        .collect()
}

pub fn load() -> Vec<PathBuf> {
    let Some(path) = state_path() else {
        return Vec::new();
    };
    let Ok(bytes) = FileSystem.read_bounded_no_follow(&path, MAX_FILE_BYTES) else {
        return Vec::new();
    };
    let paths = serde_json::from_slice::<SavedFavourites>(&bytes)
        .ok()
        .filter(|saved| saved.version == VERSION)
        .map(|saved| saved.paths)
        // Migrate the unversioned array written by older Files builds.
        .or_else(|| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
        .unwrap_or_default();
    sanitise(paths)
}

pub fn save(paths: &[PathBuf]) -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    rmac_storage::create_dir_all_private(parent)?;
    let bytes = serde_json::to_vec(&SavedFavourites {
        version: VERSION,
        paths: sanitise(paths.to_vec()),
    })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "too many sidebar favourites",
        ));
    }
    rmac_storage::atomic_write_private(&path, &bytes)
}

/// Insert or reorder a path. `index` is the insertion point in the custom
/// favourites list before removing an existing copy of the path.
pub fn insert(paths: &mut Vec<PathBuf>, path: PathBuf, index: usize) -> bool {
    if !path.is_absolute() || (!path.exists() && !paths.contains(&path)) {
        return false;
    }
    let before = paths.clone();
    let old = paths.iter().position(|candidate| candidate == &path);
    if old.is_none() && paths.len() >= MAX_FAVOURITES {
        return false;
    }
    if let Some(old) = old {
        paths.remove(old);
    }
    let index = index.saturating_sub(usize::from(old.is_some_and(|old| old < index)));
    paths.insert(index.min(paths.len()), path);
    *paths != before
}

pub fn label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_missing_targets_and_deduplication_are_supported() {
        assert_eq!(
            sanitise(vec!["relative".into(), "/tmp/a.txt".into(), "/tmp/a.txt".into(), "/gone".into()]),
            vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/gone")]
        );
    }

    #[test]
    fn insertion_reorders_without_a_duplicate() {
        let mut paths = vec!["/a".into(), "/b".into(), "/c".into()];
        assert!(insert(&mut paths, "/a".into(), 3));
        assert_eq!(paths, vec![PathBuf::from("/b"), PathBuf::from("/c"), PathBuf::from("/a")]);
    }
}
