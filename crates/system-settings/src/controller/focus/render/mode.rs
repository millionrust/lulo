//! Focus mode detail presentation, on the Mac's mode page: the mode's own
//! row with its switch, "Allow Notifications" and "Set a Schedule".

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
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying Focus change…")
                    .mb_3(),
            );
        }
        for error in [&self.focus_policy_error, &self.focus_policy_stream_error]
            .into_iter()
            .flatten()
        {
            body = body.child(note_card(rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Settings,
                error.as_ref(),
                false,
            )));
        }

        // The mode's own row: its switch turns it on until turned off, or
        // off again; "Turn On for 1 Hour" is the timed activation.
        let active = self.focus_policy_state.as_ref().is_some_and(|state| {
            state.projection.enabled && state.mode_id.as_deref() == Some(mode_id)
        });
        let switch_view = view.clone();
        let switch_mode = mode_id.to_owned();
        let hour_view = view.clone();
        let hour_mode = mode_id.to_owned();
        body = body.child(card(vec![
            row_base()
                .child(tile("icons/moon.svg", hsl(0x5e5ce6), style::ROW_ICON))
                .child(text_block(mode.name().to_owned().into(), None))
                .child(
                    Toggle::new(SharedString::from(format!("focus-on-{mode_id}")))
                        .checked(active)
                        .disabled(busy)
                        .on_click(move |on, _, cx| {
                            let mode = switch_mode.clone();
                            switch_view.update(cx, |settings, cx| {
                                if *on {
                                    settings.activate_focus(mode, 0, cx)
                                } else {
                                    settings.disable_focus(cx)
                                }
                            });
                        }),
                )
                .into_any_element(),
            value_button_row(
                "Turn on for 1 hour",
                None,
                None,
                Some(
                    push_button(
                        SharedString::from(format!("focus-hour-{mode_id}")),
                        "Turn On",
                    )
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        hour_view.update(cx, |settings, cx| {
                            settings.activate_focus(hour_mode.clone(), 60 * 60 * 1_000, cx)
                        });
                    })
                    .into_any_element(),
                ),
            ),
        ]));

        body = body.child(section_with_note(
            "Allow Notifications",
            "Notifications from the applications below will be allowed; all others will be silenced.",
            false,
        ));
        let mut allow_rows = self
            .notification_apps
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
            .collect::<Vec<_>>();
        if allow_rows.is_empty() {
            allow_rows.push(
                group_placeholder("Applications appear here after they send a notification.")
                    .into_any_element(),
            );
        }
        allow_rows.push(focus_urgent_row(&view, mode_id, mode.allow_urgent(), busy));
        body = body.child(card(allow_rows));

        body = body.child(section_with_note(
            "Set a Schedule",
            "Have this Focus turn on automatically at a set time.",
            false,
        ));
        let mut schedule_rows = configuration
            .schedules()
            .filter(|schedule| schedule.mode == parsed_mode)
            .map(|schedule| focus_schedule_row(&view, schedule, mode.name(), busy))
            .collect::<Vec<_>>();
        if schedule_rows.is_empty() {
            schedule_rows.push(group_placeholder("No Schedules").into_any_element());
        }
        let add_view = view.clone();
        let add_mode_id = mode_id.to_owned();
        schedule_rows.push(button_row(vec![push_button(
            SharedString::from(format!("focus-add-schedule-{mode_id}")),
            "Add Schedule…",
        )
        .disabled(busy)
        .on_click(move |_, _, cx| {
            add_view.update(cx, |settings, cx| {
                settings.add_focus_schedule(add_mode_id.clone(), cx);
            });
        })
        .into_any_element()]));
        body.child(card(schedule_rows))
    }
}
