//! Appearance settings lifecycle and presentation.

mod render;

use super::*;

impl Settings {
    pub(super) fn finish_theme_update(&mut self, result: std::result::Result<ThemeLoad, String>) {
        self.theme_loading = false;
        self.theme_busy = false;
        match result {
            Ok(load) => {
                self.host_appearance = load.host;
                self.theme = Some(load.theme);
                self.theme_error = None;
            }
            Err(error) => {
                self.theme_error = Some(format!("Could not update Appearance: {error}").into());
            }
        }
    }
    pub(super) fn refresh_theme(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_theme_change(&mut self, change: ThemeChange, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        let Some(theme) = &self.theme else {
            return;
        };
        let expected = theme.preferences.clone();
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(apply_theme_change_authoritatively(change, expected))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn queue_theme_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            self.theme_refresh_pending = true;
            return;
        }
        self.theme_refresh_pending = false;
        self.theme_stream_refreshing = true;
        let generation = self.theme_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.theme_stream_refreshing = false;
                if theme_stream_snapshot_is_current(
                    generation,
                    this.theme_generation,
                    this.theme_loading,
                    this.theme_busy,
                ) {
                    match result {
                        Ok(load) => this.finish_theme_update(Ok(load)),
                        Err(_) => {
                            this.theme_error =
                                Some("Could not refresh changed appearance preferences".into());
                        }
                    }
                } else {
                    this.theme_refresh_pending = true;
                }
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_theme_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_refresh_pending
            && !self.theme_loading
            && !self.theme_busy
            && !self.theme_stream_refreshing
        {
            self.queue_theme_stream_refresh(cx);
        }
    }

    #[allow(dead_code)]
    pub(super) fn appearance_accessibility_snapshot(
        &self,
    ) -> std::result::Result<
        rmac_system_settings::appearance_accessibility::AppearanceAccessibilitySnapshot,
        rmac_system_settings::appearance_accessibility::AccessibilityProjectionError,
    > {
        use rmac_system_settings::appearance_accessibility::{project_appearance, AppearanceInput};

        let error = self
            .theme_error
            .as_ref()
            .or(self.theme_store_stream_error.as_ref())
            .or(self.theme_portal_stream_error.as_ref())
            .map(|error| error.as_ref());
        project_appearance(AppearanceInput {
            theme: self.theme.as_ref(),
            host: &self.host_appearance,
            loading: self.theme_loading,
            busy: self.theme_busy,
            refreshing: self.theme_stream_refreshing,
            error,
        })
    }

    #[allow(dead_code)]
    pub(super) fn apply_appearance_accessibility_action(
        &mut self,
        action: rmac_system_settings::appearance_accessibility::AppearanceAction,
        cx: &mut Context<Self>,
    ) {
        use rmac_system_settings::appearance_accessibility::{AccentChoice, AppearanceAction};

        match action {
            AppearanceAction::Refresh => self.refresh_theme(cx),
            AppearanceAction::SetScheme(preference) => {
                self.apply_theme_change(ThemeChange::Scheme(preference), cx);
            }
            AppearanceAction::SetAccent(choice) => {
                let preference = match choice {
                    AccentChoice::Automatic => rmac_theme::AccentPreference::Automatic,
                    _ => accent_preference(
                        choice
                            .hex()
                            .expect("non-automatic accent choices have a color"),
                    ),
                };
                self.apply_theme_change(ThemeChange::Accent(preference), cx);
            }
            AppearanceAction::SetContrast(preference) => {
                self.apply_theme_change(ThemeChange::Contrast(preference), cx);
            }
            AppearanceAction::SetMotion(preference) => {
                self.apply_theme_change(ThemeChange::Motion(preference), cx);
            }
        }
    }
}
