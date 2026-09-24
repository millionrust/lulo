//! Shared shell-settings refresh and mutation authority.

use super::*;

impl Settings {
    pub(super) fn apply_shell_settings_stream_update(
        &mut self,
        update: ShellSettingsStreamUpdate,
    ) -> bool {
        self.shell_settings_loading = false;
        match update {
            ShellSettingsStreamUpdate::Snapshot(snapshot) => {
                self.shell_settings_stream_error = None;
                if self.shell_settings_busy {
                    return false;
                }
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                let spotlight_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                });
                if spotlight_changed {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(*snapshot);
                self.shell_settings_error = None;
                wallpaper_changed
            }
            ShellSettingsStreamUpdate::Unavailable(error) => {
                self.shell_settings_stream_error =
                    Some(format!("Live shell settings updates are unavailable: {error}").into());
                false
            }
        }
    }

    pub(super) fn finish_shell_settings_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
    ) -> bool {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                let spotlight_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                });
                if spotlight_changed {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
                wallpaper_changed
            }
            Err(error) => {
                self.shell_settings_error =
                    Some(format!("Could not update Desktop & Dock: {error}").into());
                false
            }
        }
    }

    pub(super) fn refresh_shell_settings(
        &mut self,
        refresh_wallpaper_preview: bool,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        self.shell_settings_loading = true;
        self.shell_settings_error = None;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(|| {
                rmac_shell_settings::ShellSettingsStore::from_environment()?.load()
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_loading = false;
                match result {
                    Ok(snapshot) => {
                        let wallpaper_changed =
                            this.shell_settings.as_ref().is_none_or(|current| {
                                current.settings.wallpaper != snapshot.settings.wallpaper
                            });
                        let spotlight_changed =
                            this.shell_settings.as_ref().is_none_or(|current| {
                                SpotlightAuthority::from_settings(&current.settings)
                                    != SpotlightAuthority::from_settings(&snapshot.settings)
                            });
                        if spotlight_changed {
                            this.spotlight_revert = None;
                        }
                        this.shell_settings = Some(snapshot);
                        this.shell_settings_error = None;
                        this.shell_settings_stream_error = None;
                        if wallpaper_changed || refresh_wallpaper_preview {
                            this.refresh_wallpaper_preview(cx);
                        }
                    }
                    Err(error) => {
                        this.shell_settings_error =
                            Some(format!("Could not refresh shell settings: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_dock_change(&mut self, change: DockChange, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = snapshot.settings.dock.clone();
        let mut next = previous.clone();
        change.clone().apply(&mut next);
        if next == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Change(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_shell_settings_mutation(result) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Save a Menu Bar pane change through the same versioned shell-settings
    /// store the menu bar watches.
    pub(super) fn apply_menu_bar_change(&mut self, change: MenuBarChange, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let mut next = snapshot.settings.clone();
        change.apply(&mut next);
        if next == snapshot.settings {
            return;
        }

        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::MenuBar(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shell_settings_mutation(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Save a Hot Corners choice through the shell-settings store that
    /// `rmac-mission-control` watches.
    pub(super) fn apply_hot_corner_change(
        &mut self,
        change: HotCornerChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        if change.corner.get(&snapshot.settings.hot_corners) == change.action {
            return;
        }

        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::HotCorner(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shell_settings_mutation(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Save "Click wallpaper to reveal desktop" through the shell-settings
    /// store that `rmac-mission-control` watches.
    pub(super) fn apply_click_wallpaper_to_reveal(
        &mut self,
        value: rmac_shell_settings::ClickWallpaperToReveal,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        if snapshot.settings.click_wallpaper_to_reveal == value {
            return;
        }

        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::ClickWallpaperToReveal(
                    value,
                ))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shell_settings_mutation(result);
                cx.notify();
            });
        })
        .detach();
    }
}
