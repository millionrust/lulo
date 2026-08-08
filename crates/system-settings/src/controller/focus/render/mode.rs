//! Focus mode detail presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn focus_mode_body(&self, mode_id: &str, cx: &Context<Self>) -> Div {
        let Some(configuration) = &self.focus_policy_config else {
            return note_card("Focus configuration is unavailable.");
        };
        let Ok(parsed_mode) = rmac_focus::ModeId::parse(mode_id) else {
            return note_card("This Focus mode is invalid.");
        };
        let Some(mode) = configuration.mode(&parsed_mode) else {
            return note_card("This Focus mode no longer exists.");
        };
        let view = cx.entity();
        let busy = self.focus_policy_busy;
        let activate_forever_view = view.clone();
        let activate_hour_view = view.clone();
        let mode_forever = mode_id.to_owned();
        let mode_hour = mode_id.to_owned();
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying Focus change…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.focus_policy_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body = body.child(card(vec![
            ListRow::new(
                SharedString::from(format!("focus-activate-{mode_id}")),
                div().child("Turn On"),
            )
            .disabled(busy)
            .on_activate(move |_, _, cx| {
                activate_forever_view.update(cx, |settings, cx| {
                    settings.activate_focus(mode_forever.clone(), 0, cx)
                });
            })
            .into_any_element(),
            ListRow::new(
                SharedString::from(format!("focus-hour-{mode_id}")),
                div().child("Turn On for 1 Hour"),
            )
            .disabled(busy)
            .on_activate(move |_, _, cx| {
                activate_hour_view.update(cx, |settings, cx| {
                    settings.activate_focus(mode_hour.clone(), 60 * 60 * 1_000, cx)
                });
            })
            .into_any_element(),
            focus_urgent_row(&view, mode_id, mode.allow_urgent(), busy),
        ]));

        body = body.child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .font_weight(rmac_ui::mac::SEMIBOLD)
                .text_color(secondary())
                .mt_3()
                .mb_1()
                .px_1()
                .child("Allowed Applications"),
        );
        body = if self.notification_apps.is_empty() {
            body.child(note_card(
                "Applications appear here after they send a notification.",
            ))
        } else {
            body.child(card(
                self.notification_apps
                    .iter()
                    .filter_map(|application| {
                        let app_id = rmac_notifications::AppId::parse(&application.app_id).ok()?;
                        let identity = self.application_identity(&application.app_id);
                        Some(focus_allowed_app_row(
                            &view,
                            mode_id,
                            &application.app_id,
                            identity
                                .map(|identity| identity.name.as_str())
                                .unwrap_or(&application.app_id),
                            identity.and_then(|identity| identity.icon.as_ref()),
                            mode.allowed_apps().contains(&app_id),
                            busy,
                        ))
                    })
                    .collect(),
            ))
        };

        body = body.child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .font_weight(rmac_ui::mac::SEMIBOLD)
                .text_color(secondary())
                .mt_3()
                .mb_1()
                .px_1()
                .child("Schedules"),
        );
        let schedules = configuration
            .schedules()
            .filter(|schedule| schedule.mode == parsed_mode)
            .collect::<Vec<_>>();
        if !schedules.is_empty() {
            body = body.child(card(
                schedules
                    .into_iter()
                    .map(|schedule| focus_schedule_row(&view, schedule, mode.name(), busy))
                    .collect(),
            ));
        }
        let add_view = view.clone();
        let add_mode_id = mode_id.to_owned();
        body.child(card(vec![ListRow::new(
            SharedString::from(format!("focus-add-schedule-{mode_id}")),
            div().text_color(accent()).child("Add Schedule…"),
        )
        .disabled(busy)
        .on_activate(move |_, _, cx| {
            add_view.update(cx, |settings, cx| {
                settings.add_focus_schedule(add_mode_id.clone(), cx);
            });
        })
        .into_any_element()]))
    }
}
