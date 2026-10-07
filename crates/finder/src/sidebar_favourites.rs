//! Versioned sidebar favourites shared by Files and the Open/Save chooser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, OnceLock};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

pub const MAX_FAVOURITES: usize = 32;
const MAX_FILE_BYTES: usize = 16 * 1024;
const VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct SavedFavourites {
    version: u32,
    paths: Vec<PathBuf>,
    #[serde(default)]
    order: Vec<FavouriteKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum FavouriteKey {
    Applications,
    Path(PathBuf),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favourites {
    pub paths: Vec<PathBuf>,
    pub order: Vec<FavouriteKey>,
}

impl Favourites {
    fn sanitised(mut self) -> Self {
        self.paths = sanitise(self.paths);
        let mut seen = BTreeSet::new();
        self.order.retain(|key| {
            seen.insert(key.clone())
                && match key {
                    FavouriteKey::Applications => true,
                    FavouriteKey::Path(path) => path.is_absolute(),
                }
        });
        self.order.truncate(MAX_FAVOURITES + 4);
        self
    }
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
    load_state().paths
}

pub fn load_state() -> Favourites {
    let Some(path) = state_path() else {
        return Favourites::default();
    };
    let Ok(bytes) = FileSystem.read_bounded_no_follow(&path, MAX_FILE_BYTES) else {
        return Favourites::default();
    };
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Favourites {
    serde_json::from_slice::<SavedFavourites>(bytes)
        .ok()
        .filter(|saved| saved.version == VERSION)
        .map(|saved| Favourites {
            paths: saved.paths,
            order: saved.order,
        })
        // Migrate the unversioned array written by older Files builds.
        .or_else(|| {
            serde_json::from_slice::<Vec<PathBuf>>(bytes)
                .ok()
                .map(|paths| Favourites {
                    paths,
                    order: Vec::new(),
                })
        })
        .unwrap_or_default()
        .sanitised()
}

pub fn save_state(state: &Favourites) -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    rmac_storage::create_dir_all_private(parent)?;
    let state = state.clone().sanitised();
    let bytes = serde_json::to_vec(&SavedFavourites {
        version: VERSION,
        paths: state.paths,
        order: state.order,
    })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "too many sidebar favourites",
        ));
    }
    rmac_storage::atomic_write_private(&path, &bytes)
}

/// Queue persistence on a single sleeping worker. Consecutive reorders are
/// coalesced so the UI never waits on storage and the last order wins.
pub fn queue_save(state: Favourites) -> std::io::Result<()> {
    static WRITER: OnceLock<Result<mpsc::Sender<Favourites>, String>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Favourites>();
        std::thread::Builder::new()
            .name("files-sidebar-save".into())
            .spawn(move || {
                while let Ok(mut state) = receiver.recv() {
                    while let Ok(newer) = receiver.try_recv() {
                        state = newer;
                    }
                    if let Err(error) = save_state(&state) {
                        eprintln!("could not save Files sidebar favourites: {error}");
                    }
                }
            })
            .map(|_| sender)
            .map_err(|error| error.to_string())
    });
    match writer {
        Ok(sender) => sender.send(state).map_err(std::io::Error::other),
        Err(error) => Err(std::io::Error::other(error.clone())),
    }
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
    #[cfg(unix)]
    fn files_missing_targets_and_deduplication_are_supported() {
        assert_eq!(
            sanitise(vec![
                "relative".into(),
                "/tmp/a.txt".into(),
                "/tmp/a.txt".into(),
                "/gone".into()
            ]),
            vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/gone")]
        );
    }

    #[test]
    #[cfg(unix)]
    fn insertion_reorders_without_a_duplicate() {
        let mut paths = vec!["/a".into(), "/b".into(), "/c".into()];
        assert!(insert(&mut paths, "/a".into(), 3));
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/b"),
                PathBuf::from("/c"),
                PathBuf::from("/a")
            ]
        );
    }

    #[test]
    fn versioned_and_legacy_documents_read_the_same_paths() {
        let v1 = br#"{"version":1,"paths":["/tmp/one","/tmp/two"]}"#;
        let legacy = br#"["/tmp/one","/tmp/two"]"#;
        assert_eq!(decode(v1), decode(legacy));
        assert_eq!(
            decode(br#"{"version":2,"paths":["/tmp/one"]}"#),
            Favourites::default()
        );
    }
}
