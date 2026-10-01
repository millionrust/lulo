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

#[cfg(test)]
pub(super) fn dedupe_absolute_directories(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    saved::sanitise(paths)
}

impl FinderView {
    pub(in crate::view) fn set_builtin_sidebar_visibility(
        &self,
        path: &Path,
        visible: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let folder = if path == self.home.join("Desktop") {
            0
        } else if path == self.home.join("Documents") {
            1
        } else if path == self.home.join("Downloads") {
            2
        } else {
            return false;
        };
        cx.defer(move |cx| {
            let _ = settings::update(|settings| match folder {
                0 => settings.sidebar.show_desktop = visible,
                1 => settings.sidebar.show_documents = visible,
                _ => settings.sidebar.show_downloads = visible,
            }, cx);
        });
        true
    }

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
        if self.set_builtin_sidebar_visibility(&path, true, cx) {
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

    pub(in crate::view) fn save_and_broadcast_favourites(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = saved::queue_save(self.favourite_extras.clone()) {
            self.operation_error = Some(format!("Could not save sidebar favourites: {error}").into());
        }
        self.rebuild_sidebar_sections(cx);
        cx.defer(settings::broadcast);
    }
}
