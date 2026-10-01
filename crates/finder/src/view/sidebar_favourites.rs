//! Files-side editing of the shared, versioned favourites preference.

use super::*;
use rmac_finder::sidebar_favourites as saved;

pub(super) fn extra_favourite_place(path: &Path) -> Place {
    Place {
        name: saved::label(path).into(),
        path: path.to_path_buf(),
        icon: if path.is_dir() {
            "icons/folder.svg"
        } else {
            "icons/file.svg"
        },
        tint: accent(),
        kind: PlaceKind::Item,
    }
}

#[cfg(test)]
pub(super) fn dedupe_absolute_directories(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    saved::sanitise(paths)
}

impl FinderView {
    fn visible_favourite_keys(&self) -> Vec<FavouriteKey> {
        self.sections
            .iter()
            .find(|section| section.title.as_ref() == self.file_words.favourites())
            .map(|section| {
                section.places.iter().map(|place| {
                    if place.kind == PlaceKind::Applications {
                        FavouriteKey::Applications
                    } else {
                        FavouriteKey::Path(place.path.clone())
                    }
                }).collect()
            })
            .unwrap_or_default()
    }

    pub(in crate::view) fn set_builtin_sidebar_visibility(
        &self,
        path: &Path,
        visible: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let folder = if path.as_os_str().is_empty() {
            3
        } else if path == self.home.join("Desktop") {
            0
        } else if path == self.home.join("Documents") {
            1
        } else if path == self.home.join("Downloads") {
            2
        } else {
            return false;
        };
        cx.defer(move |cx| {
            let _ = settings::update(
                |settings| match folder {
                    0 => settings.sidebar.show_desktop = visible,
                    1 => settings.sidebar.show_documents = visible,
                    2 => settings.sidebar.show_downloads = visible,
                    _ => settings.sidebar.show_applications = visible,
                },
                cx,
            );
        });
        true
    }

    pub(in crate::view) fn add_sidebar_favourite(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.insert_sidebar_favourite(path, self.visible_favourite_keys().len(), cx);
    }

    pub(in crate::view) fn insert_sidebar_favourite(
        &mut self,
        path: PathBuf,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let applications = path.as_os_str().is_empty();
        if !applications && (!path.is_absolute() || (!path.exists() && !self.is_removable_favourite(&path))) {
            return;
        }
        let builtin = self.set_builtin_sidebar_visibility(&path, true, cx);
        if !applications && !builtin
            && !self.favourite_extras.contains(&path)
        {
            if self.favourite_extras.len() >= saved::MAX_FAVOURITES {
                self.operation_error = Some("The sidebar can hold up to 32 custom favourites".into());
                cx.notify();
                return;
            }
            self.favourite_extras.push(path.clone());
        }
        let key = if applications { FavouriteKey::Applications } else { FavouriteKey::Path(path) };
        let mut visible = self.visible_favourite_keys();
        let old = visible.iter().position(|existing| existing == &key);
        if let Some(old) = old { visible.remove(old); }
        let slot = index.saturating_sub(usize::from(old.is_some_and(|old| old < index)));
        visible.insert(slot.min(visible.len()), key.clone());
        let mut order = visible;
        for existing in &self.favourite_order {
            if !order.contains(existing) {
                order.push(existing.clone());
            }
        }
        self.favourite_order = order;
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
            self.favourite_order.retain(|key| key != &FavouriteKey::Path(path.to_path_buf()));
            self.save_and_broadcast_favourites(cx);
        }
    }

    pub(in crate::view) fn is_removable_favourite(&self, path: &Path) -> bool {
        self.favourite_extras.iter().any(|extra| extra == path)
    }

    pub(in crate::view) fn save_and_broadcast_favourites(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = saved::queue_save(saved::Favourites {
            paths: self.favourite_extras.clone(),
            order: self.favourite_order.clone(),
        }) {
            self.operation_error =
                Some(format!("Could not save sidebar favourites: {error}").into());
        }
        self.rebuild_sidebar_sections(cx);
        cx.defer(settings::broadcast);
    }
}
