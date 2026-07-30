//! Notification policy settings lifecycle and presentation.

use super::*;

impl Settings {
    pub(super) fn finish_notifications_update(
        &mut self,
        result: std::result::Result<
            Vec<rmac_notifications_linux::center::ApplicationPolicy>,
            rmac_notifications_linux::center::Error,
        >,
    ) {
        self.notifications_loading = false;
        self.notification_busy = None;
        match result {
            Ok(applications) => {
                self.notification_apps = applications;
                self.notification_error = None;
            }
            Err(error) => {
                self.notification_error =
                    Some(format!("Could not update Notifications: {error}").into());
            }
        }
    }

    pub(super) fn apply_notification_stream_update(
        &mut self,
        update: std::result::Result<
            Vec<rmac_notifications_linux::center::ApplicationPolicy>,
            String,
        >,
    ) {
        self.notifications_loading = false;
        match update {
            Ok(applications) => {
                self.notification_apps = applications;
                self.notification_stream_error = None;
            }
            Err(error) => {
                self.notification_stream_error =
                    Some(format!("Live Notification updates unavailable: {error}").into());
            }
        }
    }

    pub(super) fn refresh_notifications(&mut self, cx: &mut Context<Self>) {
        if self.notifications_loading || self.notification_busy.is_some() {
            return;
        }
        self.notifications_loading = true;
        self.notification_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_notifications_linux::center::applications() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_notifications_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_notification_policy(
        &mut self,
        app_id: String,
        change: NotificationPolicyChange,
        cx: &mut Context<Self>,
    ) {
        if self.notifications_loading || self.notification_busy.is_some() {
            return;
        }
        let Some(application) = self
            .notification_apps
            .iter()
            .find(|application| application.app_id == app_id)
        else {
            self.notification_error = Some("That application is no longer available.".into());
            cx.notify();
            return;
        };
        let policy = notification_policy_with(application.policy, change);
        self.notification_busy = Some(app_id.clone());
        self.notification_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, applications) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_notifications_linux::center::set_policy(&app_id, policy);
                    let applications = rmac_notifications_linux::center::applications();
                    (mutation, applications)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.notification_busy = None;
                let refresh_error = match applications {
                    Ok(applications) => {
                        this.notification_apps = applications;
                        None
                    }
                    Err(error) => Some(format!("Could not refresh Notifications: {error}").into()),
                };
                this.notification_error = mutation
                    .err()
                    .map(|error| format!("Could not change Notifications: {error}").into())
                    .or(refresh_error);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_notifications(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        let refresh_view = view.clone();
        cards.push(
            div().flex().justify_end().mb_2().child(
                div()
                    .id("notifications-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(accent())
                    .when(!self.notifications_loading, |button| {
                        button
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_notifications(cx));
                            })
                    })
                    .child(if self.notifications_loading {
                        "Loading…"
                    } else {
                        "Refresh"
                    }),
            ),
        );
        if self.notifications_loading {
            cards.push(
                div()
                    .mb_3()
                    .child(Progress::indeterminate().label("Loading notification settings…")),
            );
        }
        if let Some(error) = &self.notification_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.notification_stream_error {
            cards.push(note_card(error.clone()));
        }
        if !self.notifications_loading {
            let rows = if self.notification_apps.is_empty() {
                vec![EmptyState::new("No applications yet")
                    .message("Applications appear after they send a notification")
                    .into_any_element()]
            } else {
                self.notification_apps
                    .iter()
                    .map(|application| {
                        let value = if application.policy.enabled {
                            "On"
                        } else {
                            "Off"
                        };
                        let identity = self.application_identity(&application.app_id);
                        application_nav_row(
                            &view,
                            &application.app_id,
                            identity
                                .map(|identity| identity.name.as_str())
                                .unwrap_or(&application.app_id),
                            identity.and_then(|identity| identity.icon.as_ref()),
                            value,
                            application.policy.enabled,
                            SubPage::NotificationApp {
                                app_id: application.app_id.clone(),
                            },
                        )
                    })
                    .collect()
            };
            cards.push(card(rows));
        }
        self.pane(cards)
    }

    pub(super) fn notification_app_body(&self, app_id: &str, cx: &Context<Self>) -> Div {
        let Some(application) = self
            .notification_apps
            .iter()
            .find(|application| application.app_id == app_id)
        else {
            return note_card("This application is no longer in Notification Center.");
        };
        let view = cx.entity();
        let policy = application.policy;
        let busy = self.notification_busy.as_deref() == Some(app_id);
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying notification policy…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.notification_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.notification_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body.child(card(vec![
            notification_toggle_row(
                &view,
                app_id,
                "enabled",
                "Allow notifications",
                Some("Blocks banners, sounds, badges, and history when off"),
                policy.enabled,
                busy,
                NotificationPolicyChange::Enabled,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "banners",
                "Show notification banners",
                Some("Show a banner when this application sends a notification"),
                policy.banners,
                busy || !policy.enabled,
                NotificationPolicyChange::Banners,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "sounds",
                "Play sounds for notifications",
                Some("Allow this application to play its notification sound"),
                policy.sounds,
                busy || !policy.enabled,
                NotificationPolicyChange::Sounds,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "badges",
                "Badge indicator",
                Some("Count unread notifications in the top bar"),
                policy.badges,
                busy || !policy.enabled,
                NotificationPolicyChange::Badges,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "history",
                "Notification Center history",
                Some("Turning this off immediately removes saved history"),
                policy.history,
                busy || !policy.enabled,
                NotificationPolicyChange::History,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "urgent-through-focus",
                "Allow urgent notifications through Focus",
                Some("Only notifications marked urgent may bypass an active Focus"),
                policy.urgent_through_focus,
                busy || !policy.enabled,
                NotificationPolicyChange::UrgentThroughFocus,
            ),
        ]))
        .child(note_card(
            "Lock Screen preview controls stay hidden because the shipping swaylock provider cannot securely render notification content.",
        ))
    }
}
