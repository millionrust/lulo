//! Login-item and background-service transaction lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_login_items_update(
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

    pub(super) fn queue_login_items_stream_refresh(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn run_pending_login_items_refresh(&mut self, cx: &mut Context<Self>) {
        if self.login_items_refresh_pending
            && !self.login_items_loading
            && self.login_item_busy.is_none()
            && !self.login_items_stream_refreshing
        {
            self.queue_login_items_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_login_items(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn set_login_item_enabled(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(id.clone());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_background_service_enabled(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("systemd:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_background_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn reveal_login_item(
        &mut self,
        id: String,
        background: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("reveal:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    if background {
                        rmac_login_items_linux::background_service_source(&id)
                    } else {
                        rmac_login_items_linux::autostart_source(&id)
                    }
                })
                .await;
            let result = match path {
                Ok(path) => rmac_app_launch::reveal_item(path)
                    .await
                    .map_err(|_| "the file manager could not reveal this login item".to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                this.login_items_error = result
                    .err()
                    .map(|error| format!("Could not reveal login item: {error}").into());
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn choose_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some("choose".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_desktop_entry().await;
            let preview = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move { rmac_login_items_linux::prepare_add_source(&path) })
                        .await,
                ),
                Ok(None) => None,
                Err(_) => Some(Err(rmac_login_items::Error::new(
                    rmac_login_items::ErrorKind::Unavailable,
                    "the desktop-entry chooser is unavailable",
                ))),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                match preview {
                    Some(Ok(preview)) => {
                        this.login_item_add = Some(preview);
                        this.login_items_error = None;
                    }
                    Some(Err(error)) => {
                        this.login_items_error =
                            Some(format!("Could not add login item: {error}").into());
                    }
                    None => {}
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_add_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some(preview) = self.login_item_add.clone() else {
            return;
        };
        self.login_item_busy = Some("add".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::add_source(&preview) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_add = None;
                }
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_remove_login_item(&mut self, id: String, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("prepare-remove:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::prepare_remove_autostart(&id) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                match result {
                    Ok(preview) => {
                        this.login_item_remove = Some(preview);
                        this.login_items_error = None;
                    }
                    Err(error) => {
                        this.login_items_error =
                            Some(format!("Could not prepare login item removal: {error}").into());
                    }
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remove_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some(preview) = self.login_item_remove.clone() else {
            return;
        };
        self.login_item_busy = Some("remove".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::remove_autostart(&preview) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_remove = None;
                }
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
