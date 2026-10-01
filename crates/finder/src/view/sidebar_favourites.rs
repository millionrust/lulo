//! Files-side editing of the shared, versioned favourites preference.

use super::*;
use rmac_finder::sidebar_favourites as saved;

pub(super) fn extra_favourite_place(path: &Path) -> Place {
    Place {
        name: saved::label(path).into(),
        path: path.to_path_buf(),
        icon: if path.is_dir() { "icons/folder.svg" } else { "icons/file.svg" },
        tint: accent(),
        kind: PlaceKind::Item,
    }
}

pub(super) fn load_sidebar_favourites() -> Vec<PathBuf> {
    saved::load()
}

pub(super) fn dedupe_absolute_directories(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    saved::sanitise(paths)
}

impl FinderView {
    pub(in crate::view) fn add_sidebar_favourite(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.insert_sidebar_favourite(path, self.favourite_extras.len(), cx);
    }

    pub(in crate::view) fn insert_sidebar_favourite(
        &mut self,
        path: PathBuf,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if !path.is_absolute() || !path.exists() {
            return;
        }
        if !saved::insert(&mut self.favourite_extras, path, index) {
            return;
        }
        self.save_and_broadcast_favourites(cx);
    }

    pub(in crate::view) fn remove_sidebar_favourite(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) {
        let before = self.favourite_extras.len();
        self.favourite_extras.retain(|extra| extra != path);
        if self.favourite_extras.len() != before {
            self.save_and_broadcast_favourites(cx);
        }
    }

    pub(in crate::view) fn is_removable_favourite(&self, path: &Path) -> bool {
        self.favourite_extras.iter().any(|extra| extra == path)
    }

    fn save_and_broadcast_favourites(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = saved::save(&self.favourite_extras) {
            self.operation_error = Some(format!("Could not save sidebar favourites: {error}").into());
        }
        self.rebuild_sidebar_sections(cx);
        cx.defer(settings::broadcast);
    }
}
