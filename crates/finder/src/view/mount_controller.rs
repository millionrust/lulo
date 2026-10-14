use super::*;

impl FinderView {
    /// Finder's File > Eject acts on a selected mounted volume. Keep the
    /// lookup exact so a folder inside a volume cannot eject it accidentally.
    pub(super) fn selected_ejectable_volume(&self) -> Option<PathBuf> {
        let paths = self.selected_paths();
        let selected = match paths.as_slice() {
            [selected] => selected,
            [] if !self.trash_view && !self.applications_view => &self.cwd,
            _ => return None,
        };
        self.mounts
            .iter()
            .find(|mount| mount.ejectable && mount.path == *selected)
            .map(|mount| mount.path.clone())
    }

    pub(super) fn eject_selected_volume(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_ejectable_volume() {
            self.eject_volume(path, cx);
        }
    }

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
        // The whole sidebar, so every Settings ▸ Sidebar checkbox also
        // applies to the volume that just appeared.
        self.rebuild_sidebar_sections_now();
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

    pub(super) fn eject_volume(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let unmount_path = path.clone();
            // `rmac_mounts::unmount()` shells out (e.g. to `udisksctl`);
            // GPUI's background executor is not safe to spawn child
            // processes from (LINUX-HW-07).
            let result = blocking::unblock(move || rmac_mounts::unmount(&unmount_path)).await;
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
