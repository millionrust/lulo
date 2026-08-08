//! Accessibility settings presentation.

use super::*;

mod render;

impl Settings {
    pub(super) fn finish_gtk_text_update(
        &mut self,
        result: std::result::Result<rmac_gtk_settings::Snapshot, rmac_gtk_settings::Error>,
    ) {
        self.gtk_text_loading = false;
        self.gtk_text_busy = false;
        match result {
            Ok(snapshot) => {
                self.gtk_text = Some(snapshot);
                self.gtk_text_error = None;
            }
            Err(error) => {
                self.gtk_text_error =
                    Some(format!("Could not update GTK text scaling: {error}").into());
            }
        }
    }

    pub(super) fn queue_gtk_text_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            self.gtk_text_refresh_pending = true;
            return;
        }
        self.gtk_text_refresh_pending = false;
        self.gtk_text_stream_refreshing = true;
        let generation = self.gtk_text_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.gtk_text_stream_refreshing = false;
                if gtk_text_stream_snapshot_is_current(
                    generation,
                    this.gtk_text_generation,
                    this.gtk_text_loading,
                    this.gtk_text_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.gtk_text = Some(snapshot);
                            this.gtk_text_error = None;
                        }
                        Err(_) => {
                            this.gtk_text_error =
                                Some("Could not refresh changed GTK text scaling".into());
                        }
                    }
                } else {
                    this.gtk_text_refresh_pending = true;
                }
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_gtk_text_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_refresh_pending
            && !self.gtk_text_loading
            && !self.gtk_text_busy
            && !self.gtk_text_stream_refreshing
        {
            self.queue_gtk_text_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_gtk_text(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_gtk_text_scale(&mut self, factor: f64, cx: &mut Context<Self>) {
        if self.gtk_text_loading
            || self.gtk_text_busy
            || self.gtk_text_stream_refreshing
            || !self
                .gtk_text
                .as_ref()
                .is_some_and(|snapshot| snapshot.available && snapshot.writable)
        {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_gtk_settings::set_text_scale(factor) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
