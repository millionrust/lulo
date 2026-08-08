//! Notification Center stream, catalog, snapshot, and read-state lifecycle.

use super::*;

impl NotificationCenterView {
    pub(crate) fn new(token: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();
        cx.on_release(move |_, cx| {
            if cx.has_global::<NotificationCenterService>() {
                cx.update_global::<NotificationCenterService, _>(|service, _| {
                    if service
                        .active
                        .as_ref()
                        .is_some_and(|active| active.token == token)
                    {
                        service.active = None;
                    }
                });
            }
        })
        .detach();

        let (snapshot_tx, snapshot_rx) = async_channel::bounded(4);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let watch = rmac_notifications_linux::center::watch_snapshot(snapshot_tx);
            let consume = async {
                while let Ok(update) = snapshot_rx.recv().await {
                    if this
                        .update(cx, |this, cx| this.apply_snapshot(update, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            };
            let _ = futures_lite::future::zip(watch, consume).await;
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let catalog = blocking::unblock(rmac_apps::discover).await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(catalog) = catalog {
                    this.applications = application_identities(catalog);
                }
                cx.notify();
            });
        })
        .detach();

        Self {
            token,
            snapshot: None,
            applications: BTreeMap::new(),
            stream_error: None,
            operation_error: None,
            busy: None,
            marking_read: false,
            was_active: false,
        }
    }

    fn apply_snapshot(&mut self, update: Result<Snapshot, String>, cx: &mut Context<Self>) {
        match update {
            Ok(snapshot) => {
                let should_mark_read =
                    !self.marking_read && snapshot.records.iter().any(|record| record.unread);
                self.snapshot = Some(snapshot);
                self.stream_error = None;
                if should_mark_read {
                    self.mark_all_read(cx);
                }
            }
            Err(_) => {
                self.stream_error = Some(
                    "Notification Center is unavailable; showing the last received history".into(),
                );
            }
        }
        cx.notify();
    }

    fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.marking_read = true;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(|| rmac_notifications_linux::center::mark_read(None).map(drop))
                    .await;
            let _ = this.update(cx, |this, cx| {
                this.marking_read = false;
                if result.is_err() {
                    this.operation_error = Some("Could not mark notifications as read".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.stream_error = None;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_notifications_linux::center::snapshot).await;
            let _ = this.update(cx, |this, cx| {
                this.apply_snapshot(result.map_err(|error| error.to_string()), cx)
            });
        })
        .detach();
    }
}
