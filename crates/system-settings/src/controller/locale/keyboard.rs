//! System X11 keyboard editing and rollback lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn start_x11_keyboard_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        window.focus(&focus, cx);
        self.x11_layout_editor = Some(layout_editor);
        self.x11_variant_editor = Some(variant_editor);
        self.x11_options_editor = Some(options_editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(in crate::controller) fn cancel_x11_keyboard_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.x11_layout_editor = None;
            self.x11_variant_editor = None;
            self.x11_options_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn submit_x11_keyboard(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn revert_x11_keyboard(&mut self, cx: &mut Context<Self>) {
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
