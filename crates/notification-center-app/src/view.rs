mod lifecycle;

use std::collections::BTreeSet;

use gpui::{BorrowAppContext as _, Context, SharedString, Window};
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, Snapshot};

use crate::model::{
    catalog_entries, group_records, ApplicationCatalog, ApplicationIdentity, Busy, RecordGroup,
};
use crate::NotificationCenterService;

pub(crate) struct NotificationCenterView {
    token: u64,
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
}

impl NotificationCenterView {
    /// Exact framework-neutral semantics for the future A5/A6 accessibility
    /// adapter. Pinned GPUI cannot publish this snapshot yet.
    #[allow(dead_code)]
    pub(crate) fn accessibility_snapshot(
        &self,
        time: &str,
        date: &str,
    ) -> Result<
        rmac_notification_center_app::accessibility::NotificationCenterAccessibilitySnapshot,
        rmac_notification_center_app::accessibility::AccessibilityProjectionError,
    > {
        use rmac_notification_center_app::accessibility::{
            project_notification_center, HeaderText, PanelBusy, PanelStatus,
        };

        let busy = self.busy.as_ref().and_then(|busy| match busy {
            Busy::ClearGroup(key) => Some(PanelBusy::ClearApplication(key)),
            Busy::DisableApp(app_id) => Some(PanelBusy::DisableApplication(app_id)),
            Busy::Remove(_) => None,
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

    // Deferred: the options menu that calls this is not built yet (§5.3).
    #[allow(dead_code)]
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

    // Deferred: the options menu that calls this is not built yet (§5.3).
    #[allow(dead_code)]
    fn policy(&self, app_id: &str) -> Option<rmac_notifications_store::AppPolicy> {
        self.snapshot
            .as_ref()?
            .applications
            .iter()
            .find(|application| application.app_id == app_id)
            .map(|application| application.policy)
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_global::<NotificationCenterService>() {
            let token = self.token;
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
        window.remove_window();
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

    // Deferred: the options menu that calls this is not built yet (§5.3).
    #[allow(dead_code)]
    pub(crate) fn policy_enabled(&self, app_id: &str) -> bool {
        self.policy(app_id).is_none_or(|policy| policy.enabled)
    }
}
