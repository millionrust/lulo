//! Keyboard, mouse, and trackpad settings authority.

use super::*;

mod render;
mod shortcuts;

impl Settings {
    pub(super) fn finish_input_update(
        &mut self,
        result: std::result::Result<rmac_input::Snapshot, rmac_input::Error>,
    ) {
        self.input_loading = false;
        self.input_busy = false;
        match result {
            Ok(snapshot) => {
                self.input = snapshot;
                self.input_error = None;
            }
            Err(error) => {
                self.input_error = Some(format!("Could not update Input settings: {error}").into());
            }
        }
    }
    pub(super) fn refresh_input(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_stream_refreshing = true;
        self.input_generation = self.input_generation.wrapping_add(1);
        let generation = self.input_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.input_stream_refreshing = false;
                if input_stream_snapshot_is_current(
                    generation,
                    this.input_generation,
                    this.input_loading,
                    this.input_busy,
                ) {
                    this.finish_input_update(result);
                    this.flush_input_stream_refresh(cx);
                    cx.notify();
                } else {
                    this.input_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    pub(super) fn flush_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_refresh_pending
            && !self.input_loading
            && !self.input_busy
            && !self.input_stream_refreshing
        {
            self.request_input_stream_refresh(cx);
        }
    }
    pub(super) fn apply_input_change(&mut self, change: InputChange, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || !self.input.can_configure {
            return;
        }
        let mut settings = self.input.settings.clone();
        change.apply(&mut settings);
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_input::save(&settings) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
    /// The right-aligned Refresh under Keyboard, Mouse and Trackpad.
    pub(super) fn input_refresh_button(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        footer_buttons(vec![push_button("input-refresh", "Refresh")
            .busy(self.input_busy)
            .disabled(self.input_loading || self.input_busy)
            .on_click(move |_, _, cx| {
                view.update(cx, |settings, cx| settings.refresh_input(cx));
            })
            .into_any_element()])
    }

    pub(super) fn input_unavailable_card(&self) -> Option<Div> {
        if self.input_loading {
            return Some(note_card("Loading input settings…"));
        }
        if !self.input.available || !self.input.can_configure {
            return Some(note_card(self.input.detail.clone().unwrap_or_else(|| {
                "Input settings are unavailable in this desktop session.".into()
            })));
        }
        None
    }
}
