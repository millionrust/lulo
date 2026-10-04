//! Notification, application-catalog, Focus, and lock-policy startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_apps_focus_watchers(
        cx: &mut Context<Self>,
        catalog_event_rx: async_channel::Receiver<()>,
    ) {
        let (notification_updates, notification_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_notifications_linux::center::watch_applications(notification_updates)
                    .await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = notification_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.apply_notification_stream_update(update) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            let Ok(visible) = this.update(cx, |this: &mut Settings, _| this.catalog_pane_visible())
            else {
                break;
            };
            if !visible {
                if catalog_event_rx.recv().await.is_err() {
                    break;
                }
                continue;
            }
            let Ok(events) = this.update(cx, |this: &mut Settings, _| {
                this._app_catalog_watcher
                    .is_none()
                    .then(|| this.catalog_reload.clone())
            }) else {
                break;
            };
            if let Some(events) = events {
                let watcher = blocking::unblock(move || {
                    rmac_apps::watch_catalog(move || {
                        let _ = events.try_send(());
                    })
                    .ok()
                })
                .await;
                if this
                    .update(cx, |this: &mut Settings, _| {
                        if this.catalog_pane_visible() {
                            this._app_catalog_watcher = watcher;
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
            // `rmac_apps::discover()` can shell out to `gsettings` to read
            // the active icon theme; GPUI's background executor is not
            // safe to spawn child processes from (LINUX-HW-07).
            let result = blocking::unblock(rmac_apps::discover).await;
            if let Ok(applications) = result {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.catalog_pane_visible() {
                            this.app_catalog = applications;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
            if catalog_event_rx.recv().await.is_err() {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            while catalog_event_rx.try_recv().is_ok() {}
        })
        .detach();

        let (focus_updates, focus_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_focus_linux::client::watch_settings(focus_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = focus_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.apply_focus_stream_update(update) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (lock_updates, lock_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_shortcuts::lock_settings::watch(lock_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = lock_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        this.apply_lock_policy_stream_update(update);
                        this.notify_if_current_pane(&["Lock Screen"], cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}
