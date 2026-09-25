//! Notification policy settings lifecycle and presentation.

use super::*;

/// For each app ID, the most recently arrived record's origin -- the
/// desktop-entry hint, kernel-reported scope, and sender-given name/icon
/// NC-01 (`d3a8f776`) started recording (SET-94).
fn latest_origins(
    records: &[rmac_notifications_linux::center::HistoryRecord],
) -> std::collections::BTreeMap<String, rmac_notifications_linux::origin::Origin> {
    let mut origins =
        std::collections::BTreeMap::<String, rmac_notifications_linux::origin::Origin>::new();
    for record in records {
        if record.origin.is_empty() {
            continue;
        }
        let newer = origins
            .get(&record.app_id)
            .is_none_or(|existing| record.origin.posted_unix_ms > existing.posted_unix_ms);
        if newer {
            origins.insert(record.app_id.clone(), record.origin.clone());
        }
    }
    origins
}

impl Settings {
    /// The name and icon to show for a policy row, resolved the same way
    /// Notification Center's cards are (SET-94): an installed application
    /// by the sender's hints or the app ID itself, else the name the
    /// sender gave, else `None` -- a legacy sender's raw, transient D-Bus
    /// name (`:1.1105`) is never worth showing.
    pub(super) fn notification_identity(&self, app_id: &str) -> Option<(String, Option<PathBuf>)> {
        let origin = self
            .notification_origins
            .get(app_id)
            .cloned()
            .unwrap_or_default();
        rmac_notifications_linux::origin::resolve_identity(&self.app_catalog, app_id, &origin)
    }

    pub(super) fn finish_notifications_update(
        &mut self,
        result: std::result::Result<
            rmac_notifications_linux::center::Snapshot,
            rmac_notifications_linux::center::Error,
        >,
    ) {
        self.notifications_loading = false;
        self.notification_busy = None;
        match result {
            Ok(snapshot) => {
                self.notification_origins = latest_origins(&snapshot.records);
                self.notification_apps = snapshot.applications;
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
                .spawn(async { rmac_notifications_linux::center::snapshot() })
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
            let rows: Vec<_> = self
                .notification_apps
                .iter()
                .filter_map(|application| {
                    let identity = self.notification_identity(&application.app_id);
                    if identity.is_none() && application.app_id.starts_with(':') {
                        // A legacy sender with no desktop-entry hint and no
                        // app_name of its own -- its "identity" is a raw,
                        // transient D-Bus connection name that means
                        // nothing to a person and will never come back
                        // once it disconnects. The Mac never shows
                        // anything like it (SET-94).
                        return None;
                    }
                    let name = identity
                        .as_ref()
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| application.app_id.clone());
                    let icon = identity.and_then(|(_, icon)| icon);
                    let target_view = view.clone();
                    let target = SubPage::NotificationApp {
                        app_id: application.app_id.clone(),
                    };
                    Some(large_nav_row(
                        SharedString::from(format!("notification-app-{}", application.app_id)),
                        app_icon(
                            icon.as_ref(),
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
                    ))
                })
                .collect();
            if rows.is_empty() {
                cards.push(group().child(group_placeholder(
                    "Applications appear here after they send a notification.",
                )));
            } else {
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
        let identity = self.notification_identity(app_id);
        let name = identity
            .as_ref()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| app_id.to_owned());
        let icon = identity.and_then(|(_, icon)| icon);
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
                        icon.as_ref(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::{Content, NotificationId, Priority};
    use rmac_notifications_linux::center::HistoryRecord;
    use rmac_notifications_linux::origin::Origin;

    fn record(id: u32, app_id: &str, posted_unix_ms: u64, app_name: &str) -> HistoryRecord {
        HistoryRecord {
            id: NotificationId::from_protocol(id).unwrap(),
            app_id: app_id.to_owned(),
            content: Content::new("Title", "Body").unwrap(),
            priority: Priority::Normal,
            unread: true,
            actions: Vec::new(),
            origin: Origin {
                posted_unix_ms: Some(posted_unix_ms),
                app_name: Some(app_name.to_owned()),
                ..Origin::default()
            },
        }
    }

    /// SET-94: a sender that has posted more than once keeps its most
    /// recent identity, not the first one seen or an empty one from a
    /// record with nothing recorded about its origin.
    #[test]
    fn latest_origins_keeps_the_newest_record_per_app_and_skips_empty_ones() {
        let records = vec![
            record(1, ":1.5", 1_000, "Old Name"),
            record(2, ":1.5", 5_000, "New Name"),
            HistoryRecord {
                origin: Origin::default(),
                ..record(3, ":1.9", 9_000, "unused")
            },
        ];
        let origins = latest_origins(&records);
        assert_eq!(
            origins
                .get(":1.5")
                .and_then(|origin| origin.app_name.as_deref()),
            Some("New Name")
        );
        assert!(!origins.contains_key(":1.9"));
    }
}
