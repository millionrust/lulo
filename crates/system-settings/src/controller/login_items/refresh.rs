//! Login Items stream and refresh lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_login_items_update(
        &mut self,
        result: std::result::Result<rmac_login_items::Snapshot, rmac_login_items::Error>,
    ) {
        self.login_items_loading = false;
        self.login_item_busy = None;
        match result {
            Ok(snapshot) => {
                self.login_items = Some(snapshot);
                self.login_items_error = None;
                self.login_items_stream_error = None;
            }
            Err(error) => {
                self.login_items_error =
                    Some(format!("Could not update login items: {error}").into());
            }
        }
    }

    pub(in crate::controller) fn queue_login_items_stream_refresh(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.login_items_loading
            || self.login_item_busy.is_some()
            || self.login_items_stream_refreshing
        {
            self.login_items_refresh_pending = true;
            return;
        }
        self.login_items_refresh_pending = false;
        self.login_items_stream_refreshing = true;
        let generation = self.login_items_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_items_stream_refreshing = false;
                if login_items_stream_snapshot_is_current(
                    generation,
                    this.login_items_generation,
                    this.login_items_loading,
                    this.login_item_busy.is_some(),
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.login_items = Some(snapshot);
                            this.login_items_error = None;
                            this.login_items_stream_error = None;
                        }
                        Err(_) => {
                            this.login_items_stream_error =
                                Some("Could not refresh changed Login Items state".into());
                        }
                    }
                } else {
                    this.login_items_refresh_pending = true;
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn run_pending_login_items_refresh(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.login_items_refresh_pending
            && !self.login_items_loading
            && self.login_item_busy.is_none()
            && !self.login_items_stream_refreshing
        {
            self.queue_login_items_stream_refresh(cx);
        }
    }

    pub(in crate::controller) fn refresh_login_items(&mut self, cx: &mut Context<Self>) {
        if self.login_items_loading
            || self.login_item_busy.is_some()
            || self.login_items_stream_refreshing
        {
            return;
        }
        self.login_item_busy = Some("refresh".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
