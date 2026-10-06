//! Notification Center stream, catalog, snapshot, and read-state lifecycle.

use super::*;

impl NotificationCenterView {
    pub(crate) fn new(
        token: u64,
        previous_window: Option<rmac_compositor::WindowId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();
        cx.on_release(move |_, cx| {
            crate::clear_active_panel(token, cx);
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

        let (first_frame_tx, first_frame_rx) = async_channel::bounded(1);
        window.on_next_frame(move |_, _| {
            let _ = first_frame_tx.try_send(());
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            if first_frame_rx.recv().await.is_err() {
                return;
            }
            let catalog = blocking::unblock(|| {
                rmac_apps::discover()
                    .map(|catalog| ApplicationCatalog::new(catalog_entries(catalog)))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(catalog) = catalog {
                    this.applications = catalog;
                }
                cx.notify();
            });
        })
        .detach();

        // The widgets added here, read from the desktop's saved state. The
        // Weather face shows the cache the wallpaper process keeps fresh.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            use rmac_desktop::widgets::{WidgetKind, WidgetSize};
            let (widgets, data) = blocking::unblock(|| {
                let widgets = rmac_desktop::settings::load()
                    .map(|settings| {
                        settings
                            .notification_center_widgets()
                            .copied()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut data = rmac_desktop_widgets::WidgetData::default();
                if widgets
                    .iter()
                    .any(|widget| widget.kind == WidgetKind::Battery)
                {
                    let (battery, none) = rmac_desktop_widgets::read_battery();
                    data.battery = battery;
                    data.no_battery = none;
                }
                if widgets
                    .iter()
                    .any(|widget| widget.kind == WidgetKind::Weather)
                {
                    data.weather = Some(rmac_desktop_widgets::read_weather(false));
                }
                if widgets.iter().any(|widget| {
                    widget.kind == WidgetKind::Calendar && widget.size == WidgetSize::Medium
                }) {
                    data.calendar = rmac_desktop_widgets::read_calendar_events(3);
                }
                (widgets, data)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.widgets = widgets;
                this.widget_data = data;
                cx.notify();
            });
        })
        .detach();

        // The Clock widget's second hand.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            let alive = this.update(cx, |this, cx| {
                if this
                    .widgets
                    .iter()
                    .any(|widget| widget.kind == rmac_desktop::widgets::WidgetKind::Clock)
                {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();

        // Relative times ("now", "5m ago") advance while the panel is open.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();

        Self {
            focus,
            token,
            previous_window,
            snapshot: None,
            applications: ApplicationCatalog::default(),
            expanded: BTreeSet::new(),
            hovered: None,
            stream_error: None,
            operation_error: None,
            busy: None,
            marking_read: false,
            was_active: false,
            a11y_active_last_frame: false,
            widgets: Vec::new(),
            widget_data: rmac_desktop_widgets::WidgetData::default(),
            scale_factor: window.scale_factor(),
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
}
