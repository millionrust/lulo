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

    pub(super) fn finish_screen_reader_toggle_update(
        &mut self,
        result: std::result::Result<rmac_screen_reader::Snapshot, rmac_screen_reader::Error>,
    ) {
        self.screen_reader_toggle_loading = false;
        self.screen_reader_toggle_busy = false;
        match result {
            Ok(snapshot) => {
                self.screen_reader_toggle = Some(snapshot);
                self.screen_reader_toggle_error = None;
            }
            Err(error) => {
                self.screen_reader_toggle_error =
                    Some(format!("Could not update the screen reader: {error}").into());
            }
        }
    }

    pub(super) fn queue_screen_reader_toggle_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.screen_reader_toggle_loading
            || self.screen_reader_toggle_busy
            || self.screen_reader_toggle_stream_refreshing
        {
            self.screen_reader_toggle_refresh_pending = true;
            return;
        }
        self.screen_reader_toggle_refresh_pending = false;
        self.screen_reader_toggle_stream_refreshing = true;
        let generation = self.screen_reader_toggle_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_screen_reader::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.screen_reader_toggle_stream_refreshing = false;
                if screen_reader_toggle_stream_snapshot_is_current(
                    generation,
                    this.screen_reader_toggle_generation,
                    this.screen_reader_toggle_loading,
                    this.screen_reader_toggle_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.screen_reader_toggle = Some(snapshot);
                            this.screen_reader_toggle_error = None;
                        }
                        Err(_) => {
                            this.screen_reader_toggle_error =
                                Some("Could not refresh the changed screen reader setting".into());
                        }
                    }
                } else {
                    this.screen_reader_toggle_refresh_pending = true;
                }
                this.run_pending_screen_reader_toggle_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_screen_reader_toggle_refresh(&mut self, cx: &mut Context<Self>) {
        if self.screen_reader_toggle_refresh_pending
            && !self.screen_reader_toggle_loading
            && !self.screen_reader_toggle_busy
            && !self.screen_reader_toggle_stream_refreshing
        {
            self.queue_screen_reader_toggle_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_screen_reader_toggle(&mut self, cx: &mut Context<Self>) {
        if self.screen_reader_toggle_loading
            || self.screen_reader_toggle_busy
            || self.screen_reader_toggle_stream_refreshing
        {
            return;
        }
        self.screen_reader_toggle_busy = true;
        self.screen_reader_toggle_generation = self.screen_reader_toggle_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_screen_reader::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_screen_reader_toggle_update(result);
                this.run_pending_screen_reader_toggle_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Set GSettings' `screen-reader-enabled` and start or stop Orca to
    /// match, the same call the Screen Reader shortcut makes.
    pub(super) fn set_screen_reader_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.screen_reader_toggle_loading
            || self.screen_reader_toggle_busy
            || self.screen_reader_toggle_stream_refreshing
            || !self
                .screen_reader_toggle
                .as_ref()
                .is_some_and(|snapshot| snapshot.available && snapshot.writable)
        {
            return;
        }
        self.screen_reader_toggle_busy = true;
        self.screen_reader_toggle_generation = self.screen_reader_toggle_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_screen_reader::set_enabled(enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_screen_reader_toggle_update(result);
                this.run_pending_screen_reader_toggle_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
