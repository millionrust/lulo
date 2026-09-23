use super::*;

impl FinderView {
    pub(super) fn refresh_mounts(&mut self, cx: &mut Context<Self>) {
        self.mount_generation = self.mount_generation.wrapping_add(1);
        let generation = self.mount_generation;
        #[cfg(target_os = "linux")]
        if self.mount_watch_health.unavailable && self.operation_error.is_none() {
            self.operation_error = Some(MOUNT_WATCH_UNAVAILABLE_MESSAGE.into());
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_mounts::discover() })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.mount_generation != generation {
                    return;
                }
                match result {
                    Ok(mounts) => this.apply_mount_snapshot(mounts, cx),
                    Err(error) => {
                        this.operation_error =
                            Some(format!("Could not refresh mounted volumes: {error}").into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn apply_mount_snapshot(&mut self, mounts: Vec<rmac_mounts::Mount>, cx: &mut Context<Self>) {
        let disappeared = disappeared_mount_roots(&self.mounts, &mounts);
        let appeared = mounts.iter().any(|new| {
            !self.mounts.iter().any(|old| {
                old.identity == new.identity
                    && old.path == new.path
                    && old.ejectable == new.ejectable
            })
        });
        self.mounts = mounts;
        self.rebuild_location_places();
        if appeared {
            let _ = rmac_sound::play(rmac_sound::Cue::Mount);
        }
        if !disappeared.is_empty() {
            let _ = rmac_sound::play(rmac_sound::Cue::Unmount);
        }
        if disappeared.is_empty() {
            cx.notify();
            return;
        }

        let current_lost = directory_state::lies_under_any(&self.cwd, &disappeared);
        self.back
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        self.fwd
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        for tab in &mut self.tabs {
            tab.back
                .retain(|path| !directory_state::lies_under_any(path, &disappeared));
            tab.fwd
                .retain(|path| !directory_state::lies_under_any(path, &disappeared));
            if directory_state::lies_under_any(&tab.cwd, &disappeared) {
                tab.cwd = self.home.clone();
                tab.identity = None;
                tab.back.clear();
                tab.fwd.clear();
            }
        }
        self.clipboard
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        if self.clipboard.is_empty() {
            self.clip_cut = false;
        }
        self.thumbs
            .retain(|path, _| !directory_state::lies_under_any(path, &disappeared));
        if self
            .open_with
            .as_ref()
            .is_some_and(|picker| directory_state::lies_under_any(&picker.path, &disappeared))
        {
            self.open_with = None;
        }
        if self.quick_look.as_ref().is_some_and(|panel| {
            panel
                .paths
                .iter()
                .any(|path| directory_state::lies_under_any(path, &disappeared))
        }) {
            if let Some(panel) = self.quick_look.take() {
                panel.handle.close(cx);
            }
        }

        if current_lost {
            self.cwd = self.home.clone();
            self.cwd_identity = None;
            self.back.clear();
            self.fwd.clear();
            self.selected.clear();
            self.anchor = None;
            self.renaming = None;
            self.info = None;
            self.menu_at = None;
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.cwd = self.cwd.clone();
                tab.identity = None;
                tab.back.clear();
                tab.fwd.clear();
            }
            self.operation_notice =
                Some("A mounted volume disconnected; affected tabs returned to Home".into());
            self.persist_finder_state();
            self.reload(cx);
        } else {
            self.operation_notice =
                Some("A mounted volume disconnected; stale locations were removed".into());
            self.persist_finder_state();
            cx.notify();
        }
    }

    fn rebuild_location_places(&mut self) {
        let host = self
            .home
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string());
        let mut places = Vec::new();
        let icloud = self
            .home
            .join("Library/Mobile Documents/com~apple~CloudDocs");
        if icloud.is_dir() {
            places.push(Place {
                name: "iCloud Drive".into(),
                path: icloud,
                icon: "icons/cloud.svg",
                tint: accent(),
                kind: PlaceKind::Item,
            });
        }
        places.extend([Place {
            name: host.into(),
            path: self.home.clone(),
            icon: "icons/house.svg",
            tint: drive_gray(),
            kind: PlaceKind::Item,
        }]);
        places.extend(self.mounts.iter().map(|mount| Place {
            name: mount.name.clone().into(),
            path: mount.path.clone(),
            icon: "icons/hard-drive.svg",
            tint: drive_gray(),
            kind: if mount.ejectable {
                PlaceKind::Volume
            } else {
                PlaceKind::Item
            },
        }));
        #[cfg(target_os = "linux")]
        places.push(Place {
            name: self.file_words.bin().into(),
            path: PathBuf::new(),
            icon: "icons/trash-2.svg",
            tint: accent(),
            kind: PlaceKind::Trash,
        });
        if let Some(locations) = self
            .sections
            .iter_mut()
            .find(|section| section.title.as_ref() == "Locations")
        {
            locations.places = places;
        }
    }

    pub(super) fn eject_volume(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let unmount_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { rmac_mounts::unmount(&unmount_path) })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                match result {
                    Ok(()) => {
                        this.operation_error = None;
                        this.refresh_mounts(cx);
                        return;
                    }
                    Err(error) => {
                        this.operation_error =
                            Some(format!("Could not eject volume: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
