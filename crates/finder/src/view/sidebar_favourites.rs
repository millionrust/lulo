//! Editable sidebar Favourites: a folder dropped on the Favourites header is
//! added, a small remove button takes one back out, and the extra list
//! persists across launches and is shared by every window (Finder's own
//! sidebar favourites are a single preference, not a per-window setting).

use super::*;
use rmac_storage::{Backend as _, FileSystem};

const MAX_FAVOURITES: usize = 32;
const MAX_FILE_BYTES: usize = 16 * 1024;

/// A sidebar row for a user-added favourite. Built-in favourites (Downloads,
/// Documents, Desktop, …) come from `rmac_finder::places` instead and are
/// never removable this way.
pub(super) fn extra_favourite_place(path: &Path) -> Place {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Place {
        name: name.into(),
        path: path.to_path_buf(),
        icon: "icons/folder.svg",
        tint: accent(),
        kind: PlaceKind::Item,
    }
}

fn favourites_path() -> Option<PathBuf> {
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

/// Extra favourites saved from an earlier session, existing folders only.
pub(super) fn load_sidebar_favourites() -> Vec<PathBuf> {
    let Some(path) = favourites_path() else {
        return Vec::new();
    };
    let Ok(bytes) = FileSystem.read_bounded_no_follow(&path, MAX_FILE_BYTES) else {
        return Vec::new();
    };
    let paths: Vec<PathBuf> = serde_json::from_slice(&bytes).unwrap_or_default();
    dedupe_absolute_directories(paths)
}

/// Keeps existing, absolute, once-each paths, capped the same way a save is:
/// what a load reads back is always a save's own output already put through
/// this, so this is also the round-trip contract [`save_sidebar_favourites`]
/// relies on.
fn dedupe_absolute_directories(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| path.is_absolute() && seen.insert(path.clone()))
        .take(MAX_FAVOURITES)
        .collect()
}

pub(super) fn save_sidebar_favourites(paths: &[PathBuf]) {
    let Some(path) = favourites_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if rmac_storage::create_dir_all_private(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(paths) else {
        return;
    };
    if bytes.len() > MAX_FILE_BYTES {
        return;
    }
    if let Err(error) = rmac_storage::atomic_write_private(&path, &bytes) {
        eprintln!("could not save the Files sidebar favourites: {error}");
    }
}

impl FinderView {
    /// A folder dropped on the Favourites header, as dragging one into
    /// Finder's own Favourites does.
    pub(in crate::view) fn add_sidebar_favourite(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() || self.favourite_extras.contains(&path) {
            return;
        }
        if self.favourite_extras.len() >= MAX_FAVOURITES {
            self.operation_error = Some("The sidebar can hold up to 32 favourites".into());
            cx.notify();
            return;
        }
        self.favourite_extras.push(path.clone());
        if let Some(index) = self.favourites_section_index() {
            self.sections[index]
                .places
                .push(extra_favourite_place(&path));
        }
        save_sidebar_favourites(&self.favourite_extras);
        cx.notify();
    }

    /// The small × next to a favourite added this way: takes it back out of
    /// the sidebar without touching the folder itself.
    pub(in crate::view) fn remove_sidebar_favourite(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) {
        let before = self.favourite_extras.len();
        self.favourite_extras.retain(|extra| extra != path);
        if self.favourite_extras.len() == before {
            return;
        }
        if let Some(index) = self.favourites_section_index() {
            self.sections[index]
                .places
                .retain(|place| place.path != path);
        }
        save_sidebar_favourites(&self.favourite_extras);
        cx.notify();
    }

    pub(in crate::view) fn is_removable_favourite(&self, path: &Path) -> bool {
        self.favourite_extras.iter().any(|extra| extra == path)
    }

    fn favourites_section_index(&self) -> Option<usize> {
        let label = self.file_words.favourites();
        self.sections
            .iter()
            .position(|section| section.title.as_ref() == label)
    }
}
