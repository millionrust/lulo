mod lifecycle;

use std::collections::BTreeSet;

use gpui::{Context, FocusHandle, SharedString, Window};
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, Snapshot};

use crate::model::{
    catalog_entries, group_records, ApplicationCatalog, ApplicationIdentity, Busy, RecordGroup,
};

pub(crate) struct NotificationCenterView {
    pub(crate) focus: FocusHandle,
    pub(crate) token: u64,
    previous_window: Option<rmac_compositor::WindowId>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) applications: ApplicationCatalog,
    /// Group keys whose stacks are expanded ("Show Less" collapses them).
    pub(crate) expanded: BTreeSet<String>,
    /// The card or stack under the pointer, which shows its × button.
    pub(crate) hovered: Option<String>,
    pub(crate) stream_error: Option<SharedString>,
    pub(crate) operation_error: Option<SharedString>,
    pub(crate) busy: Option<Busy>,
    pub(crate) marking_read: bool,
    was_active: bool,
    pub(crate) a11y_active_last_frame: bool,
    /// Widgets added to Notification Centre from the Edit Widgets gallery.
    pub(crate) widgets: Vec<rmac_desktop::widgets::Widget>,
    pub(crate) widget_data: rmac_desktop_widgets::WidgetData,
    /// `Window::scale_factor()`, refreshed every `render()` so a card's
    /// app icon (`rmac_ui::svg_icon`) rasterizes for the surface's real
    /// physical pixels.
    pub(crate) scale_factor: f32,
    /// The open "<App> Notifications" right-click menu and the app it is for.
    pub(crate) app_menu: Option<(rmac_ui::ContextMenuState, AppMenuTarget)>,
}

/// The sending application a card's right-click menu acts on: every service
/// application ID of its displayed stack, so muting or turning off covers
/// the whole stack.
#[derive(Clone)]
pub(crate) struct AppMenuTarget {
    pub(crate) name: SharedString,
    pub(crate) app_ids: Vec<String>,
}

gpui::actions!(
    notification_center,
    [
        MuteForHour,
        MuteForToday,
        TurnOffApp,
        OpenNotificationSettings
    ]
);

/// Unix seconds now.
pub(crate) fn unix_now() -> u64 {
    rmac_notifications_linux::origin::unix_ms_now() / 1000
}

/// The end of "Mute for Today": the next local midnight after `now`.
pub(crate) fn end_of_today(now: u64) -> u64 {
    use chrono::{Local, TimeZone as _};
    let fallback = now + 24 * 3600;
    let Some(local) = Local.timestamp_opt(now as i64, 0).single() else {
        return fallback;
    };
    local
        .date_naive()
        .succ_opt()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map(|midnight| midnight.timestamp().max(0) as u64)
        .filter(|end| *end > now)
        .unwrap_or(fallback)
}

impl NotificationCenterView {
    /// Framework-neutral accessibility contract; render publishes the
    /// corresponding controls through GPUI's AccessKit nodes.
    #[allow(dead_code)]
    pub(crate) fn accessibility_snapshot(
        &self,
        time: &str,
        date: &str,
    ) -> Result<
        crate::accessibility::NotificationCenterAccessibilitySnapshot,
        crate::accessibility::AccessibilityProjectionError,
    > {
        use crate::accessibility::{
            project_notification_center, HeaderText, PanelBusy, PanelStatus,
        };

        let busy = self.busy.as_ref().and_then(|busy| match busy {
            Busy::ClearGroup(key) => Some(PanelBusy::ClearApplication(key)),
            Busy::DisableApp(app_id) => Some(PanelBusy::DisableApplication(app_id)),
            Busy::Remove(_) | Busy::MuteApp(_) => None,
            Busy::Invoke(notification, selection) => Some(PanelBusy::Invoke {
                notification: *notification,
                selection: *selection,
            }),
        });
        project_notification_center(
            self.snapshot.as_ref(),
            HeaderText { time, date },
            PanelStatus {
                stream_error: self.stream_error.as_ref().map(|error| error.as_ref()),
                operation_error: self.operation_error.as_ref().map(|error| error.as_ref()),
                busy,
                marking_read: self.marking_read,
            },
            |app_id| self.identity(app_id).name.as_ref().to_owned(),
        )
    }

    /// Clears one displayed stack, which may hold several service
    /// application IDs (each process that sent through the legacy bus).
    pub(crate) fn clear_group(
        &mut self,
        key: String,
        app_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::ClearGroup(key.clone()));
        self.expanded.remove(&key);
        self.hovered = None;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                app_ids.iter().try_for_each(|app_id| {
                    rmac_notifications_linux::center::clear(Some(app_id.as_str())).map(drop)
                })
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                if result.is_err() {
                    this.operation_error =
                        Some("Could not clear Notification Center history".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Removes a single card, like its × on macOS.
    pub(crate) fn remove(&mut self, id: NotificationId, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Remove(id));
        self.hovered = None;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(move || rmac_notifications_linux::center::remove(id).map(drop))
                    .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                if result.is_err() {
                    this.operation_error = Some("Could not clear that notification".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a card's "<App> Notifications" menu at the pointer.
    pub(crate) fn open_app_menu(
        &mut self,
        app_id: &str,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self
            .groups()
            .into_iter()
            .find(|group| group.app_ids.iter().any(|candidate| candidate == app_id))
            .map(|group| AppMenuTarget {
                name: group.identity.name.clone(),
                app_ids: group.app_ids.clone(),
            })
        else {
            return;
        };
        let state = rmac_ui::ContextMenuState::open(position, &self.focus, window, cx);
        self.app_menu = Some((state, target));
        self.hovered = None;
        cx.notify();
    }

    pub(crate) fn dismiss_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut state = self.app_menu.take().map(|(state, _)| state);
        if rmac_ui::ContextMenuState::dismiss(&mut state, window, cx) {
            cx.notify();
        }
    }

    /// The menu's app, closing the menu.
    fn take_menu_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AppMenuTarget> {
        let target = self.app_menu.as_ref().map(|(_, target)| target.clone());
        self.dismiss_app_menu(window, cx);
        target
    }

    /// "Mute for 1 Hour" / "Mute for Today": the app's notifications keep
    /// arriving in Notification Centre, without a banner or a sound, until
    /// `until` (Unix seconds).
    pub(crate) fn mute_menu_app(&mut self, until: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.take_menu_target(window, cx) {
            self.update_policies(
                target.app_ids,
                move |policy| rmac_notifications_store::AppPolicy {
                    muted_until: until,
                    ..policy
                },
                Busy::MuteApp,
                "Could not mute notifications for that application",
                cx,
            );
        }
    }

    /// "Turn Off": the app's notifications stop and its cards leave the
    /// Center, as on macOS; Settings ▸ Notifications turns them back on.
    pub(crate) fn turn_off_menu_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.take_menu_target(window, cx) {
            self.update_policies(
                target.app_ids,
                |policy| rmac_notifications_store::AppPolicy {
                    enabled: false,
                    ..policy
                },
                Busy::DisableApp,
                "Could not turn off notifications for that application",
                cx,
            );
        }
    }

    /// "Notification Settings…": System Settings' Notifications pane.
    pub(crate) fn open_notification_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_app_menu(window, cx);
        cx.background_spawn(async {
            match std::process::Command::new("rmac-system-settings")
                .args(["--pane", "notifications"])
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.wait();
                }
                Err(error) => eprintln!("could not open Notification Settings: {error}"),
            }
        })
        .detach();
        self.dismiss(window, cx);
    }

    fn update_policies(
        &mut self,
        app_ids: Vec<String>,
        change: impl Fn(rmac_notifications_store::AppPolicy) -> rmac_notifications_store::AppPolicy,
        busy: fn(String) -> Busy,
        failure: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = app_ids.first().cloned() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        let policies: Vec<_> = app_ids
            .iter()
            .map(|app_id| {
                let policy = change(self.policy(app_id).unwrap_or_default());
                (app_id.clone(), policy)
            })
            .collect();
        self.busy = Some(busy(first));
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                policies.iter().try_for_each(|(app_id, policy)| {
                    rmac_notifications_linux::center::set_policy(app_id, *policy).map(drop)
                })
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                if result.is_err() {
                    this.operation_error = Some(failure.into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn invoke_action(
        &mut self,
        id: NotificationId,
        selection: ActionSelection,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Invoke(id, selection));
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // GPUI 0.2.2 does not expose the initiating Wayland seat/serial,
            // so the Linux gate must prove focus behavior without a fabricated
            // activation token.
            let result = blocking::unblock(move || {
                rmac_notifications_linux::center::invoke(id, selection, None)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                if result.is_err() {
                    this.operation_error = Some(
                        "That notification action is no longer available or could not be delivered"
                            .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn policy(&self, app_id: &str) -> Option<rmac_notifications_store::AppPolicy> {
        self.snapshot
            .as_ref()?
            .applications
            .iter()
            .find(|application| application.app_id == app_id)
            .map(|application| application.policy)
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::clear_active_panel(self.token, cx);
        window.remove_window();
    }

    pub(crate) fn dismiss_and_restore_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss(window, cx);
        if let Some(previous_window) = self.previous_window {
            cx.spawn(async move |_, _| {
                let action = rmac_compositor::Action::FocusWindow {
                    window: previous_window,
                };
                if let Err(error) = rmac_compositor_system::execute_action(&action).await {
                    eprintln!("could not return focus from Notification Centre: {error:?}");
                }
            })
            .detach();
        }
    }

    pub(crate) fn groups(&self) -> Vec<RecordGroup<'_>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        group_records(&snapshot.records, &self.applications)
    }

    pub(crate) fn identity(&self, app_id: &str) -> ApplicationIdentity {
        self.applications.identity(app_id)
    }

    pub(crate) fn toggle_expanded(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_owned());
        }
        self.hovered = None;
        cx.notify();
    }

    pub(crate) fn set_hovered(&mut self, target: &str, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(target.to_owned())
        } else if self.hovered.as_deref() == Some(target) {
            None
        } else {
            return;
        };
        if self.hovered != next {
            self.hovered = next;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::end_of_today;

    #[test]
    fn mute_for_today_ends_at_the_next_local_midnight() {
        use chrono::{Local, TimeZone as _, Timelike as _};
        let now = Local
            .with_ymd_and_hms(2026, 10, 20, 15, 30, 0)
            .earliest()
            .unwrap()
            .timestamp() as u64;
        let end = end_of_today(now);
        assert!(end > now && end - now <= 24 * 3600);
        let local = Local.timestamp_opt(end as i64, 0).single().unwrap();
        assert_eq!((local.hour(), local.minute()), (0, 0));
        // A moment before midnight still mutes until that midnight.
        assert_eq!(end_of_today(end - 1), end);
    }
}
