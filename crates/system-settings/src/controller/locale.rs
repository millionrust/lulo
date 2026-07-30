//! Language, region, and system keyboard transaction lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_locale_update(
        &mut self,
        result: std::result::Result<rmac_locale::Snapshot, rmac_locale::Error>,
    ) {
        self.locale_loading = false;
        self.locale_busy = false;
        match result {
            Ok(snapshot) => {
                self.locale = Some(snapshot);
                self.locale_error = None;
                self.locale_stream_error = None;
            }
            Err(error) => {
                self.locale_error =
                    Some(format!("Could not update language and region: {error}").into());
            }
        }
    }

    pub(super) fn queue_locale_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.locale_loading || self.locale_busy || self.locale_stream_refreshing {
            self.locale_refresh_pending = true;
            return;
        }
        self.locale_refresh_pending = false;
        self.locale_stream_refreshing = true;
        let generation = self.locale_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.locale_stream_refreshing = false;
                if locale_stream_snapshot_is_current(
                    generation,
                    this.locale_generation,
                    this.locale_loading,
                    this.locale_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.locale = Some(snapshot);
                            this.locale_error = None;
                            this.locale_stream_error = None;
                        }
                        Err(_) => {
                            this.locale_stream_error =
                                Some("Could not refresh changed language and region state".into());
                        }
                    }
                } else {
                    this.locale_refresh_pending = true;
                }
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_locale_refresh(&mut self, cx: &mut Context<Self>) {
        if self.locale_refresh_pending
            && !self.locale_loading
            && !self.locale_busy
            && !self.locale_stream_refreshing
        {
            self.queue_locale_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_loading || self.locale_busy {
            return;
        }
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_locale_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.locale_editor.is_some()
            || self.region_editor.is_some()
            || self.x11_layout_editor.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let language = snapshot.language().to_owned();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(language)
                .placeholder("en_US.UTF-8")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.locale_editor = Some(editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_locale_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.locale_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.locale_editor, &self.locale) else {
            return;
        };
        let language = editor.read(cx).value().trim().to_owned();
        let next = match snapshot.preview_language(&language) {
            Ok(assignments) => assignments
                .iter()
                .map(rmac_locale::Assignment::encoded)
                .collect::<Vec<_>>(),
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.apply_locale_assignments(next, snapshot.clone(), cx);
    }

    pub(super) fn start_region_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.region_editor.is_some()
            || self.locale_editor.is_some()
            || self.x11_layout_editor.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let region = snapshot.region_locale().to_owned();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(region)
                .placeholder("en_IN.UTF-8")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.region_editor = Some(editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_region_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.region_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_region(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.region_editor, &self.locale) else {
            return;
        };
        let region = editor.read(cx).value().trim().to_owned();
        let next = match snapshot.preview_region(&region) {
            Ok(assignments) => assignments
                .iter()
                .map(rmac_locale::Assignment::encoded)
                .collect::<Vec<_>>(),
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.apply_locale_assignments(next, snapshot.clone(), cx);
    }

    pub(super) fn apply_locale_assignments(
        &mut self,
        next: Vec<String>,
        previous: rmac_locale::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_locale(&next) })
                .await;
            let rollback = result
                .as_ref()
                .ok()
                .map(|applied| rmac_locale::LocaleRollback::new(&previous, applied));
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(rollback) = rollback {
                    this.locale_editor = None;
                    this.region_editor = None;
                    this.locale_revert = Some(rollback);
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(rollback) = self.locale_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::restore_locale(&rollback) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.locale_revert = None;
                    this.locale_editor = None;
                    this.region_editor = None;
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_x11_keyboard_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.x11_layout_editor.is_some()
            || self.locale_editor.is_some()
            || self.region_editor.is_some()
            || self.input.keyboard_layout_authority
                != rmac_input::KeyboardLayoutAuthority::SystemLocaled
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let layout = snapshot.x11_layout.clone();
        let variant = snapshot.x11_variant.clone();
        let options = snapshot.x11_options.clone();
        let layout_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(layout)
                .placeholder("us,de")
        });
        let variant_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(variant)
                .placeholder(",nodeadkeys")
        });
        let options_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(options)
                .placeholder("grp:ctrl_space_toggle")
        });
        let focus = layout_editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.x11_layout_editor = Some(layout_editor);
        self.x11_variant_editor = Some(variant_editor);
        self.x11_options_editor = Some(options_editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_x11_keyboard_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.x11_layout_editor = None;
            self.x11_variant_editor = None;
            self.x11_options_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(layout_editor), Some(variant_editor), Some(options_editor), Some(snapshot)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
            &self.locale,
        ) else {
            return;
        };
        let layout = layout_editor.read(cx).value().trim().to_owned();
        let variant = variant_editor.read(cx).value().trim().to_owned();
        let options = options_editor.read(cx).value().trim().to_owned();
        let keyboard = match snapshot.preview_x11_keyboard(&layout, &variant, &options) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let previous = snapshot.clone();
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_x11_keyboard(&keyboard) })
                .await;
            let rollback = result
                .as_ref()
                .ok()
                .map(|applied| rmac_locale::KeyboardRollback::new(&previous, applied));
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(rollback) = rollback {
                    this.x11_layout_editor = None;
                    this.x11_variant_editor = None;
                    this.x11_options_editor = None;
                    this.x11_keyboard_revert = Some(rollback);
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(rollback) = self.x11_keyboard_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::restore_x11_keyboard(&rollback) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.x11_keyboard_revert = None;
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
