use std::collections::BTreeMap;
use std::process::Command;

use gpui::{Context, SharedString, Window};
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, Snapshot};

use crate::model::{
    application_identities, fallback_app_name, ApplicationIdentity, Busy, RecordGroup,
};

pub(crate) struct NotificationCenterView {
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) applications: BTreeMap<String, ApplicationIdentity>,
    pub(crate) stream_error: Option<SharedString>,
    pub(crate) operation_error: Option<SharedString>,
    pub(crate) busy: Option<Busy>,
    pub(crate) marking_read: bool,
    was_active: bool,
}

impl NotificationCenterView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                window.remove_window();
                cx.quit();
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

    pub(crate) fn clear(&mut self, app_id: Option<String>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(match &app_id {
            Some(app_id) => Busy::ClearApp(app_id.clone()),
            None => Busy::ClearAll,
        });
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_notifications_linux::center::clear(app_id.as_deref()).map(drop)
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

    pub(crate) fn disable_app(&mut self, app_id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(policy) = self.policy(&app_id) else {
            self.operation_error = Some("That application is no longer available".into());
            cx.notify();
            return;
        };
        self.busy = Some(Busy::DisableApp(app_id.clone()));
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_notifications_linux::center::set_policy(
                    &app_id,
                    rmac_notifications_store::AppPolicy {
                        enabled: false,
                        ..policy
                    },
                )
                .map(drop)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                if result.is_err() {
                    this.operation_error =
                        Some("Could not turn off notifications for that application".into());
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
        window.remove_window();
        cx.quit();
    }

    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(|| {
                let executable = std::env::current_exe()?.with_file_name("rmac-system-settings");
                Command::new(executable)
                    .arg("--pane")
                    .arg("notifications")
                    .spawn()
                    .map(drop)
            })
            .await;
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.operation_error = Some("Could not open Notification Settings".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn groups(&self) -> Vec<RecordGroup<'_>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut positions = BTreeMap::<&str, usize>::new();
        let mut groups: Vec<RecordGroup<'_>> = Vec::new();
        for record in &snapshot.records {
            if let Some(position) = positions.get(record.app_id.as_str()).copied() {
                groups[position].records.push(record);
            } else {
                positions.insert(record.app_id.as_str(), groups.len());
                groups.push(RecordGroup {
                    app_id: &record.app_id,
                    records: vec![record],
                });
            }
        }
        groups
    }

    pub(crate) fn identity(&self, app_id: &str) -> ApplicationIdentity {
        self.applications
            .get(app_id)
            .cloned()
            .unwrap_or_else(|| ApplicationIdentity {
                name: fallback_app_name(app_id).into(),
                icon: None,
            })
    }

    pub(crate) fn policy_enabled(&self, app_id: &str) -> bool {
        self.policy(app_id).is_none_or(|policy| policy.enabled)
    }
}
