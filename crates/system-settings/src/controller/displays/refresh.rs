//! Display snapshot, stream, refresh, and mutation lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_display_update(
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

    pub(in crate::controller) fn request_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn flush_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_refresh_pending
            && !self.display_loading
            && !self.display_busy
            && self.display_confirmation.is_none()
        {
            self.request_display_stream_refresh(cx);
        }
    }

    pub(in crate::controller) fn refresh_displays(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn apply_display_change(
        &mut self,
        change: DisplayChange,
        cx: &mut Context<Self>,
    ) {
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
}
