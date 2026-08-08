//! Wallpaper preview, chooser, mutation, rollback, and pane authority.

use super::*;

mod render;

impl Settings {
    pub(super) fn refresh_wallpaper_preview(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.shell_settings.as_ref() else {
            self.wallpaper_preview_loading = false;
            self.wallpaper_preview_error = Some("Wallpaper settings are unavailable".into());
            return;
        };
        let (selection, _) =
            wallpaper_selection(&snapshot.settings.wallpaper, &self.wallpaper_target);
        let watched_paths = rmac_wallpaper::parse_source(selection.source.as_deref())
            .ok()
            .and_then(|source| rmac_wallpaper::file_path(&source).map(PathBuf::from))
            .into_iter()
            .collect::<Vec<_>>();
        self.wallpaper_preview_generation = self.wallpaper_preview_generation.wrapping_add(1);
        let generation = self.wallpaper_preview_generation;
        self.wallpaper_preview_loading = true;
        self.wallpaper_preview_error = None;
        self.wallpaper_preview_watch_error = None;
        self._wallpaper_preview_watcher = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (events_tx, events_rx) = async_channel::bounded(1);
            let (result, watcher) = blocking::unblock(move || {
                let watcher = rmac_wallpaper_image::watch_files(&watched_paths, events_tx);
                (render_wallpaper_preview(&selection), watcher)
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.wallpaper_preview_generation != generation {
                    return;
                }
                this.wallpaper_preview_loading = false;
                match result {
                    Ok(preview) => {
                        this.wallpaper_preview = Some(preview);
                        this.wallpaper_preview_error = None;
                    }
                    Err(error) => {
                        this.wallpaper_preview_error =
                            Some(format!("Could not preview this wallpaper: {error}").into());
                    }
                }
                let watching = match watcher {
                    Ok(watcher) => {
                        let watching = watcher.is_some();
                        this._wallpaper_preview_watcher = watcher;
                        this.wallpaper_preview_watch_error = None;
                        watching
                    }
                    Err(_) => {
                        this.wallpaper_preview_watch_error = Some(
                            "Live updates for the selected wallpaper file are unavailable".into(),
                        );
                        false
                    }
                };
                if watching {
                    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                        let event = events_rx.recv().await;
                        let _ = this.update(cx, |this: &mut Settings, cx| {
                            if this.wallpaper_preview_generation != generation {
                                return;
                            }
                            match event {
                                Ok(rmac_wallpaper_image::FileWatchEvent::Changed) => {
                                    this.refresh_wallpaper_preview(cx);
                                }
                                Ok(rmac_wallpaper_image::FileWatchEvent::Failed { .. })
                                | Err(_) => {
                                    this._wallpaper_preview_watcher = None;
                                    this.wallpaper_preview_watch_error = Some(
                                        "Live updates for the selected wallpaper file stopped"
                                            .into(),
                                    );
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_wallpaper_target(
        &mut self,
        target: WallpaperTarget,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_busy || self.wallpaper_target == target {
            return;
        }
        self.wallpaper_target = target;
        self.wallpaper_error = None;
        self.refresh_wallpaper_preview(cx);
    }

    pub(super) fn apply_wallpaper_change(
        &mut self,
        target: WallpaperTarget,
        change: WallpaperChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = snapshot.settings.wallpaper.clone();
        let mut next = previous.clone();
        change.clone().apply(&target, &mut next);
        if next == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Wallpaper { target, change })
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, Some(previous));
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_wallpaper_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<rmac_shell_settings::WallpaperSettings>,
    ) {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                if self
                    .shell_settings
                    .as_ref()
                    .is_some_and(|current| current.settings.dock != snapshot.settings.dock)
                {
                    self.shell_settings_revert = None;
                }
                if self.shell_settings.as_ref().is_some_and(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                }) {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.wallpaper_revert = previous;
                self.wallpaper_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
            }
            Err(error) => {
                self.wallpaper_error = Some(format!("Could not update Wallpaper: {error}").into());
            }
        }
    }

    pub(super) fn choose_wallpaper_file(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let target = self.wallpaper_target.clone();
        let fit = wallpaper_selection(&snapshot.settings.wallpaper, &target)
            .0
            .fit;
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_wallpaper_file().await;
            let validated = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            blocking::unblock(move || validate_wallpaper_choice(path, fit)).await
                        })
                        .await,
                ),
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(source)) => this.apply_wallpaper_change(
                        target,
                        WallpaperChange::Source(Some(source)),
                        cx,
                    ),
                    Some(Err(error)) => {
                        this.wallpaper_error = Some(error.into());
                    }
                    None => this.refresh_shell_settings(true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_wallpaper_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.wallpaper_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreWallpaper(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, None);
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
