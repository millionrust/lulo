//! Locale stream and refresh lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_locale_update(
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

    pub(in crate::controller) fn queue_locale_stream_refresh(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn run_pending_locale_refresh(&mut self, cx: &mut Context<Self>) {
        if self.locale_refresh_pending
            && !self.locale_loading
            && !self.locale_busy
            && !self.locale_stream_refreshing
        {
            self.queue_locale_stream_refresh(cx);
        }
    }

    pub(in crate::controller) fn refresh_locale(&mut self, cx: &mut Context<Self>) {
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
}
