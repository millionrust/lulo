//! Lock Screen policy and immediate-session-lock lifecycle.

mod render;

use super::*;

fn lock_policy_choice_row(
    id: SharedString,
    icon: &'static str,
    title: &'static str,
    detail: &'static str,
    selected: bool,
    disabled: bool,
) -> ListRow {
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(tile(icon, secondary_foreground, style::ROW_ICON))
        .child(
            div()
                .v_flex()
                .flex_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(foreground)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(secondary_foreground)
                        .child(detail),
                ),
        )
        .when(selected, |row| {
            row.child(glyph("icons/check.svg", 14.0, on_accent()))
        });
    ListRow::new(ElementId::from(id), content)
        .selected(selected)
        .disabled(disabled)
        .h(px(60.0))
        .px_3()
}

impl Settings {
    pub(super) fn finish_lock_policy_update(
        &mut self,
        result: std::result::Result<
            rmac_shortcuts::lock_settings::Snapshot,
            rmac_shortcuts::lock_settings::Error,
        >,
    ) {
        self.lock_policy_loading = false;
        self.lock_policy_busy = false;
        match result {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_error = None;
            }
            Err(error) => {
                self.lock_policy_error =
                    Some(format!("Could not update Lock Screen: {error}").into());
            }
        }
    }

    pub(super) fn apply_lock_policy_stream_update(
        &mut self,
        update: std::result::Result<rmac_shortcuts::lock_settings::Snapshot, String>,
    ) {
        self.lock_policy_loading = false;
        match update {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_stream_error = None;
            }
            Err(error) => {
                self.lock_policy_stream_error =
                    Some(format!("Live Lock Screen updates unavailable: {error}").into());
            }
        }
    }

    pub(super) fn refresh_lock_policy(&mut self, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_loading = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_shortcuts::lock_settings::settings() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_lock_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_lock_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_suspend_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_suspend_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_lock_screen(&mut self, cx: &mut Context<Self>) {
        if self.lock_request_busy {
            return;
        }
        self.lock_request_busy = true;
        self.lock_request_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_shortcuts::lock::request() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.lock_request_busy = false;
                this.lock_request_error = result
                    .err()
                    .map(|error| format!("Could not lock this session: {error}").into());
                cx.notify();
            });
        })
        .detach();
    }
}
