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
        let mut cards = vec![header_card(
            tile26("icons/bell.svg", hsl(0xff3b30)),
            "Notifications",
            "Customise how notifications appear, whether they play a sound and which applications can send them.",
            None,
        )];
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
            cards.push(section_header("Application Notifications"));
            if self.notification_apps.is_empty() {
                cards.push(group().child(group_placeholder(
                    "Applications appear here after they send a notification.",
                )));
            } else {
                let rows = self
                    .notification_apps
                    .iter()
                    .map(|application| {
                        let identity = self.application_identity(&application.app_id);
                        let name = identity
                            .map(|identity| identity.name.clone())
                            .unwrap_or_else(|| application.app_id.clone());
                        let target_view = view.clone();
                        let target = SubPage::NotificationApp {
                            app_id: application.app_id.clone(),
                        };
                        large_nav_row(
                            SharedString::from(format!("notification-app-{}", application.app_id)),
                            app_icon(
                                identity.and_then(|identity| identity.icon.as_ref()),
                                "icons/app-window.svg",
                                secondary(),
                                style::LARGE_ICON,
                            ),
                            name,
                            Some(subtitle_text(notification_summary(&application.policy))),
                            None,
                            move |_, cx| {
                                let target = target.clone();
                                target_view.update(cx, |settings, cx| settings.push(target, cx));
                            },
                        )
                    })
                    .collect();
                cards.push(card(rows));
            }
        }
        let refresh_view = view.clone();
        cards.push(footer_buttons(vec![push_button(
            "notifications-refresh",
            "Refresh",
        )
        .disabled(self.notifications_loading || self.notification_busy.is_some())
        .on_click(move |_, _, cx| {
            refresh_view.update(cx, |settings, cx| settings.refresh_notifications(cx));
        })
        .into_any_element()]));
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
        let identity = self.application_identity(app_id);
        let name = identity
            .map(|identity| identity.name.clone())
            .unwrap_or_else(|| app_id.to_owned());
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying notification policy…")
                    .mb_3(),
            );
        }
        for error in [&self.notification_error, &self.notification_stream_error]
            .into_iter()
            .flatten()
        {
            body = body.child(note_card(rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Settings,
                error.as_ref(),
                false,
            )));
        }
        let change = |change: fn(bool) -> NotificationPolicyChange| {
            let view = view.clone();
            let app = app_id.to_owned();
            move |value: bool, _: &mut Window, cx: &mut App| {
                view.update(cx, |settings, cx| {
                    settings.apply_notification_policy(app.clone(), change(value), cx);
                });
            }
        };
        // "Allow notifications" beside the application's own icon.
        let allow = change(NotificationPolicyChange::Enabled);
        body = body.child(
            group().child(
                large_row(
                    app_icon(
                        identity.and_then(|identity| identity.icon.as_ref()),
                        "icons/app-window.svg",
                        secondary(),
                        style::LARGE_ICON,
                    ),
                    "Allow notifications",
                    Some(subtitle_text(name)),
                )
                .child(
                    Toggle::new(ElementId::from(SharedString::from(format!(
                        "notification-enabled-{app_id}"
                    ))))
                    .checked(policy.enabled)
                    .disabled(busy)
                    .on_click(move |value, window, cx| allow(*value, window, cx)),
                ),
            ),
        );
        let enabled = !busy && policy.enabled;
        // Where notifications appear: the Mac's preview pictures with a
        // checkbox under each. The Lock Screen one is left out because the
        // swaylock provider cannot render notification content.
        let banners = change(NotificationPolicyChange::Banners);
        let history = change(NotificationPolicyChange::History);
        let banners_on = policy.banners;
        let history_on = policy.history;
        body = body.child(
            group()
                .flex_row()
                .justify_around()
                .pt(px(20.0))
                .pb(px(16.0))
                .child(notification_preview(
                    SharedString::from(format!("notification-banners-{app_id}")),
                    "Desktop",
                    false,
                    banners_on,
                    enabled,
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        banners(!banners_on, window, cx)
                    }),
                ))
                .child(notification_preview(
                    SharedString::from(format!("notification-history-{app_id}")),
                    "Notification Centre",
                    true,
                    history_on,
                    enabled,
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        history(!history_on, window, cx)
                    }),
                )),
        );
        body.child(card(vec![
            switch_row(
                SharedString::from(format!("notification-urgent-{app_id}")),
                "Time Sensitive Notifications",
                None,
                policy.urgent_through_focus,
                enabled,
                change(NotificationPolicyChange::UrgentThroughFocus),
            ),
            switch_row(
                SharedString::from(format!("notification-badges-{app_id}")),
                "Badge application icon",
                None,
                policy.badges,
                enabled,
                change(NotificationPolicyChange::Badges),
            ),
            switch_row(
                SharedString::from(format!("notification-sounds-{app_id}")),
                "Play sound for notification",
                None,
                policy.sounds,
                enabled,
                change(NotificationPolicyChange::Sounds),
            ),
        ]))
    }
}

/// The Mac's list subtitle: "Off", or what the application may do, in the
/// Mac's order — "Badges, Sounds, Desktop, and Time Sensitive".
fn notification_summary(policy: &rmac_notifications_store::AppPolicy) -> String {
    if !policy.enabled {
        return "Off".into();
    }
    let parts: Vec<&str> = [
        (policy.badges, "Badges"),
        (policy.sounds, "Sounds"),
        (policy.banners, "Desktop"),
        (policy.urgent_through_focus, "Time Sensitive"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    match parts.as_slice() {
        [] => "Notification Centre".into(),
        [one] => (*one).into(),
        [first, second] => format!("{first}, {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// An 88 × 58 picture of where a notification appears (a banner on the
/// desktop, or the Notification Centre stack), its name and a checkbox.
fn notification_preview(
    id: SharedString,
    title: &'static str,
    stack: bool,
    checked: bool,
    enabled: bool,
    toggle: FormHandler,
) -> impl IntoElement {
    let white = gpui::hsla(0.0, 0.0, 1.0, 0.9);
    let art = div()
        .relative()
        .w(px(88.0))
        .h(px(58.0))
        .rounded(px(4.0))
        .bg(gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(hsl(0x8fa4c9), 0.0),
            gpui::linear_color_stop(hsl(0x9b82b8), 1.0),
        ))
        .when(!stack, |art| {
            art.child(
                div()
                    .absolute()
                    .right(px(4.0))
                    .top(px(5.0))
                    .w(px(22.0))
                    .h(px(6.0))
                    .rounded(px(2.0))
                    .bg(white),
            )
        })
        .when(stack, |art| {
            art.child(
                div()
                    .absolute()
                    .right(px(4.0))
                    .top(px(5.0))
                    .w(px(22.0))
                    .v_flex()
                    .gap(px(2.0))
                    .children((0..5).map(|_| div().h(px(5.0)).rounded(px(1.0)).bg(white))),
            )
        });
    div()
        .id(id)
        .v_flex()
        .items_center()
        .when(enabled, |preview| {
            preview
                .cursor_pointer()
                .on_click(move |_, window, cx| toggle(window, cx))
        })
        .when(!enabled, |preview| preview.opacity(0.5))
        .child(art)
        .child(
            div()
                .mt(px(10.0))
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .text_color(label())
                .child(title),
        )
        .child(div().mt(px(7.0)).child(form_checkbox(checked)))
}
