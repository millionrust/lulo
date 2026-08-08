//! Display configuration, confirmation, and presentation authority.

use super::*;

mod render;

fn display_choice_row(
    id: SharedString,
    title: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    disabled: bool,
) -> ListRow {
    let has_subtitle = subtitle.is_some();
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    let mut text = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(foreground)
            .child(title),
    );
    if let Some(subtitle) = subtitle {
        text = text.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary_foreground)
                .child(subtitle),
        );
    }
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .child(text)
        .when(selected, |row| {
            row.child(glyph("icons/check.svg", 14.0, on_accent()))
        });
    ListRow::new(ElementId::from(id), content)
        .selected(selected)
        .disabled(disabled)
        .h(px(if has_subtitle { 60.0 } else { 44.0 }))
        .px_3()
}

impl Settings {
    pub(super) fn finish_display_update(
        &mut self,
        result: std::result::Result<rmac_display::Snapshot, rmac_display::Error>,
    ) {
        self.display_loading = false;
        self.display_busy = false;
        match result {
            Ok(snapshot) => {
                self.display = snapshot;
                self.display_error = None;
            }
            Err(error) => {
                self.display_error = Some(format!("Could not update Displays: {error}").into());
            }
        }
    }

    pub(super) fn request_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            self.display_refresh_pending = true;
            return;
        }
        self.display_refresh_pending = false;
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.display_generation == generation
                    && !this.display_loading
                    && !this.display_busy
                    && this.display_confirmation.is_none()
                {
                    this.finish_display_update(result);
                    cx.notify();
                } else {
                    this.display_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    pub(super) fn flush_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_refresh_pending
            && !self.display_loading
            && !self.display_busy
            && self.display_confirmation.is_none()
        {
            self.request_display_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_displays(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            return;
        }
        self.display_refresh_pending = false;
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                if succeeded {
                    this.display_generation = this.display_generation.wrapping_add(1);
                    this.display_confirmation = None;
                }
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_display_change(&mut self, change: DisplayChange, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_configure
        {
            return;
        }
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { change.apply() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(applied) => {
                        let confirmed = applied.snapshot.clone();
                        this.finish_display_update(Ok(applied.snapshot));
                        this.begin_display_confirmation(applied.baseline, confirmed, cx);
                    }
                    Err(error) => {
                        this.finish_display_update(Err(error));
                        this.flush_display_stream_refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn begin_display_confirmation(
        &mut self,
        baseline: rmac_display::Snapshot,
        applied: rmac_display::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        self.display_confirmation = Some(DisplayConfirmation {
            baseline,
            applied,
            generation,
            seconds_remaining: DISPLAY_CONFIRMATION_SECONDS,
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            for remaining in (0..DISPLAY_CONFIRMATION_SECONDS).rev() {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let update = this.update(cx, |this: &mut Settings, cx| {
                    let current = this.display_confirmation.as_ref().is_some_and(|pending| {
                        pending.generation == generation && this.display_generation == generation
                    });
                    if !current {
                        return false;
                    }
                    if remaining == 0 {
                        this.revert_display_change(cx);
                    } else if let Some(pending) = &mut this.display_confirmation {
                        pending.seconds_remaining = remaining;
                        cx.notify();
                    }
                    true
                });
                if !matches!(update, Ok(true)) || remaining == 0 {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn keep_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_persist {
            return;
        }
        let Some(pending) = self.display_confirmation.clone() else {
            return;
        };
        let Some(primary) = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .or_else(|| {
                self.display
                    .outputs
                    .iter()
                    .find(|output| output.logical.is_some())
            })
            .map(|output| output.id.clone())
        else {
            self.display_error = Some("No enabled display can be saved as Main".into());
            cx.notify();
            return;
        };
        let layout = match rmac_display::current_layout(&self.display, &primary) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error =
                    Some(format!("Could not prepare display layout: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_confirmation = None;
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(snapshot) => {
                        this.finish_display_update(Ok(snapshot));
                        this.flush_display_stream_refresh(cx);
                    }
                    Err(error) => {
                        this.display_busy = false;
                        this.display_error =
                            Some(format!("Could not save display layout: {error}").into());
                        this.begin_display_confirmation(pending.baseline, pending.applied, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_primary_display(&mut self, output: String, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_persist
        {
            return;
        }
        let layout = match rmac_display::current_layout(&self.display, &output) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error = Some(format!("Could not select Main display: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        let Some(pending) = self.display_confirmation.take() else {
            return;
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_display::restore_snapshot(&pending.baseline, &pending.applied)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
