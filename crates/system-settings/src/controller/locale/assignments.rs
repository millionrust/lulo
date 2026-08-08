//! Language and regional-format editing lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn start_locale_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::controller) fn cancel_locale_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.locale_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn submit_locale(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn start_region_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::controller) fn cancel_region_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.region_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn submit_region(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn apply_locale_assignments(
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

    pub(in crate::controller) fn revert_locale(&mut self, cx: &mut Context<Self>) {
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
}
