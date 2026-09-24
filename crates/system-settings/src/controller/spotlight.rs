//! Spotlight policy, recent-history, shortcut, and pane authority.

use super::*;

mod render;

impl Settings {
    pub(super) fn apply_spotlight_change(
        &mut self,
        change: SpotlightChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = SpotlightAuthority::from_settings(&snapshot.settings);
        let mut next = snapshot.settings.clone();
        change.clone().apply(&mut next);
        if SpotlightAuthority::from_settings(&next) == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Spotlight(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, Some(previous)) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_spotlight_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<SpotlightAuthority>,
    ) -> bool {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                self.shell_settings = Some(snapshot);
                self.spotlight_revert = previous;
                self.spotlight_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
                wallpaper_changed
            }
            Err(error) => {
                self.spotlight_error = Some(format!("Could not update Spotlight: {error}").into());
                false
            }
        }
    }

    pub(super) fn choose_search_exclusion(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_search_exclusion().await;
            let validated = match choice {
                Ok(Some(path)) => {
                    Some(blocking::unblock(move || validate_search_exclusion(path)).await)
                }
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(path)) => {
                        this.apply_spotlight_change(SpotlightChange::AddExclusion(path), cx)
                    }
                    Some(Err(error)) => this.spotlight_error = Some(error.into()),
                    None => this.refresh_shell_settings(false, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_spotlight_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.spotlight_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreSpotlight(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, None) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy {
            return;
        }
        self.recent_history_confirmation = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
    }

    pub(super) fn cancel_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if !self.recent_history_busy && self.recent_history_confirmation {
            self.recent_history_confirmation = false;
            cx.notify();
        }
    }

    pub(super) fn confirm_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy || !self.recent_history_confirmation {
            return;
        }
        self.recent_history_confirmation = false;
        self.recent_history_busy = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_recent_documents::Store::from_environment().and_then(|store| store.clear())
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.recent_history_busy = false;
                match result {
                    Ok(_) => {
                        this.recent_history_notice =
                            Some("Recent document history was cleared for Spotlight.".into());
                    }
                    Err(_) => {
                        this.spotlight_error =
                            Some("Could not clear recent document history.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_shortcut_status_update(
        &mut self,
        result: std::result::Result<rmac_shortcuts::BackendStatus, rmac_shortcuts::Error>,
    ) {
        self.shortcut_status_loading = false;
        match result {
            Ok(status) => {
                self.shortcut_status = Some(status);
                self.shortcut_status_error = None;
            }
            Err(_) => {
                self.shortcut_status_error =
                    Some("The session shortcut broker has not reported its backend".into());
            }
        }
    }

    pub(super) fn refresh_shortcut_status(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_status_loading {
            return;
        }
        self.shortcut_status_loading = true;
        self.shortcut_status_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_shortcuts::backend_status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shortcut_status_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn configure_global_shortcuts(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_configuration_busy
            || !shortcut_configuration_available(self.shortcut_status.as_ref())
        {
            return;
        }
        self.shortcut_configuration_busy = true;
        self.shortcut_configuration_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(rmac_shortcuts::request_shortcut_configuration).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shortcut_configuration_busy = false;
                if result.is_err() {
                    this.shortcut_configuration_error = Some(
                        "Could not open global shortcut configuration. The live session broker or portal may be unavailable."
                            .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }
}
