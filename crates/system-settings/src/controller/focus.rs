//! Focus policy, scheduling, and presentation authority.

use super::*;

impl Settings {
    pub(super) fn finish_focus_update(
        &mut self,
        result: std::result::Result<FocusLoad, rmac_focus_linux::client::Error>,
    ) {
        self.focus_policy_loading = false;
        self.focus_policy_busy = false;
        match result {
            Ok(load) => {
                self.focus_policy_config = Some(load.configuration);
                self.focus_policy_state = Some(load.state);
                self.focus_policy_error = None;
            }
            Err(error) => {
                self.focus_policy_error = Some(format!("Could not update Focus: {error}").into());
            }
        }
    }

    pub(super) fn apply_focus_stream_update(
        &mut self,
        update: std::result::Result<rmac_focus_linux::client::SettingsSnapshot, String>,
    ) {
        self.focus_policy_loading = false;
        match update {
            Ok(update) => {
                self.focus_policy_config = Some(update.configuration);
                self.focus_policy_state = Some(update.state);
                self.focus_policy_stream_error = None;
            }
            Err(error) => {
                self.focus_policy_stream_error =
                    Some(format!("Live Focus updates unavailable: {error}").into());
            }
        }
    }

    pub(super) fn refresh_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_loading = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx.background_executor().spawn(async { load_focus() }).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn activate_focus(
        &mut self,
        mode_id: String,
        duration_ms: u64,
        cx: &mut Context<Self>,
    ) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_focus_linux::client::activate(&mode_id, duration_ms);
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, None);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn disable_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async {
                    let mutation = rmac_focus_linux::client::disable();
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, None);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn replace_focus_configuration(
        &mut self,
        configuration: rmac_focus::Config,
        cx: &mut Context<Self>,
    ) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        let previous_configuration = self.focus_policy_config.clone();
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        self.focus_policy_config = Some(configuration.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_focus_linux::client::replace_configuration(&configuration);
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, previous_configuration);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_focus_mutation(
        &mut self,
        mutation: std::result::Result<
            rmac_focus_linux::client::Snapshot,
            rmac_focus_linux::client::Error,
        >,
        reload: std::result::Result<FocusLoad, rmac_focus_linux::client::Error>,
        rollback: Option<rmac_focus::Config>,
    ) {
        self.focus_policy_busy = false;
        let mutation_failed = mutation.is_err();
        let reload_error = match reload {
            Ok(load) => {
                self.focus_policy_config = Some(load.configuration);
                self.focus_policy_state = Some(load.state);
                None
            }
            Err(error) => {
                if mutation_failed {
                    self.focus_policy_config = rollback;
                }
                Some(format!("Could not refresh Focus: {error}").into())
            }
        };
        self.focus_policy_error = mutation
            .err()
            .map(|error| format!("Could not change Focus: {error}").into())
            .or(reload_error);
    }

    pub(super) fn set_focus_urgent(
        &mut self,
        mode_id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(mode_id) = rmac_focus::ModeId::parse(mode_id) else {
            self.focus_policy_error = Some("The Focus mode is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_mode_urgent(configuration, &mode_id, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus mode.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn set_focus_allowed_app(
        &mut self,
        mode_id: String,
        app_id: String,
        allowed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let (Ok(mode_id), Ok(app_id)) = (
            rmac_focus::ModeId::parse(mode_id),
            rmac_notifications::AppId::parse(app_id),
        ) else {
            self.focus_policy_error = Some("The Focus application rule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_allowed_app(configuration, &mode_id, app_id, allowed) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus mode.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn set_focus_schedule_enabled(
        &mut self,
        schedule_id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_schedule_enabled(configuration, &schedule_id, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn add_focus_schedule(&mut self, mode_id: String, cx: &mut Context<Self>) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(mode_id) = rmac_focus::ModeId::parse(mode_id) else {
            self.focus_policy_error = Some("The Focus mode is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::create_schedule(configuration, &mode_id) {
            Ok((configuration, schedule_id)) => {
                let schedule_id = schedule_id.as_str().to_owned();
                self.replace_focus_configuration(configuration, cx);
                self.push(SubPage::FocusSchedule { schedule_id }, cx);
            }
            Err(rmac_focus_settings::Error::Limit) => {
                self.focus_policy_error =
                    Some("Focus already has the maximum number of schedules.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not create a Focus schedule.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn set_focus_schedule_day(
        &mut self,
        schedule_id: String,
        day: rmac_focus::Weekday,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_schedule_day(configuration, &schedule_id, day, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(rmac_focus_settings::Error::Invalid) => {
                self.focus_policy_error =
                    Some("A Focus schedule must include at least one day.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn set_focus_schedule_time(
        &mut self,
        schedule_id: String,
        start: bool,
        minute: u16,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        let result = if start {
            rmac_focus_settings::set_schedule_start(configuration, &schedule_id, minute)
        } else {
            rmac_focus_settings::set_schedule_end(configuration, &schedule_id, minute)
        };
        match result {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(rmac_focus_settings::Error::Invalid) => {
                self.focus_policy_error =
                    Some("A Focus schedule needs different start and end times.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn remove_focus_schedule(&mut self, schedule_id: String, cx: &mut Context<Self>) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::remove_schedule(configuration, &schedule_id) {
            Ok(configuration) => {
                self.replace_focus_configuration(configuration, cx);
                self.nav.pop();
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not remove that Focus schedule.".into());
                cx.notify();
            }
        }
    }
    pub(super) fn render_focus(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        let refresh_view = view.clone();
        cards.push(
            div().flex().justify_end().mb_2().child(
                div()
                    .id("focus-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(accent())
                    .when(!self.focus_policy_loading, |button| {
                        button
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |settings, cx| settings.refresh_focus(cx));
                            })
                    })
                    .child(if self.focus_policy_loading {
                        "Loading…"
                    } else {
                        "Refresh"
                    }),
            ),
        );
        if self.focus_policy_loading || self.focus_policy_busy {
            cards.push(div().mb_3().child(Progress::indeterminate().label(
                if self.focus_policy_busy {
                    "Applying Focus change…"
                } else {
                    "Loading Focus…"
                },
            )));
        }
        if let Some(error) = &self.focus_policy_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(state) = &self.focus_policy_state {
            let active = state.projection.enabled;
            let mut status = state
                .projection
                .mode_name
                .clone()
                .unwrap_or_else(|| "Off".into());
            if matches!(
                state.source,
                Some(rmac_focus::ActivationSource::Schedule(_))
            ) {
                status.push_str(" · Scheduled");
            }
            let mut current = row_base()
                .child(tile(
                    "icons/moon.svg",
                    if active { accent() } else { secondary() },
                    22.0,
                ))
                .child(text_block("Current Focus".into(), Some(status.into())));
            match focus_current_action(state.source.as_ref()) {
                FocusCurrentAction::TurnOffManual => {
                    let turn_off_view = view.clone();
                    current = current.child(
                        div()
                            .id("focus-turn-off")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                turn_off_view.update(cx, |settings, cx| settings.disable_focus(cx));
                            })
                            .child("Turn Off"),
                    );
                }
                FocusCurrentAction::EditSchedule(schedule_id) => {
                    let edit_view = view.clone();
                    let schedule_id = schedule_id.as_str().to_owned();
                    current = current.child(
                        div()
                            .id("focus-edit-active-schedule")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                edit_view.update(cx, |settings, cx| {
                                    settings.push(
                                        SubPage::FocusSchedule {
                                            schedule_id: schedule_id.clone(),
                                        },
                                        cx,
                                    );
                                });
                            })
                            .child("Edit Schedule"),
                    );
                }
                FocusCurrentAction::None => {}
            }
            cards.push(card(vec![current.into_any_element()]));
        }
        if let Some(configuration) = &self.focus_policy_config {
            let active_mode = self
                .focus_policy_state
                .as_ref()
                .and_then(|state| state.mode_id.as_deref());
            let mode_rows = configuration
                .modes()
                .map(|mode| {
                    nav_row(
                        view.clone(),
                        "icons/moon.svg",
                        if active_mode == Some(mode.id().as_str()) {
                            accent()
                        } else {
                            secondary()
                        },
                        mode.name().to_owned().into(),
                        (active_mode == Some(mode.id().as_str())).then(|| "Active".into()),
                        SubPage::FocusMode {
                            mode_id: mode.id().as_str().to_owned(),
                        },
                    )
                })
                .collect();
            cards.push(card(mode_rows));

            let schedules = configuration.schedules().collect::<Vec<_>>();
            if schedules.is_empty() {
                cards.push(note_card(
                    "No Focus schedules are configured. Schedule creation stays hidden until its complete day-and-time editor is available.",
                ));
            } else {
                cards.push(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .mt_3()
                        .mb_1()
                        .px_1()
                        .child("Schedules"),
                );
                cards.push(card(
                    schedules
                        .into_iter()
                        .map(|schedule| {
                            let mode_name = configuration
                                .mode(&schedule.mode)
                                .map(rmac_focus::Mode::name)
                                .unwrap_or("Unknown Focus");
                            focus_schedule_row(&view, schedule, mode_name, self.focus_policy_busy)
                        })
                        .collect(),
                ));
            }
        }
        self.pane(cards)
    }

    pub(super) fn focus_mode_body(&self, mode_id: &str, cx: &Context<Self>) -> Div {
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

    pub(super) fn focus_schedule_body(&self, schedule_id: &str, cx: &Context<Self>) -> Div {
        let Some(configuration) = &self.focus_policy_config else {
            return note_card("Focus configuration is unavailable.");
        };
        let Ok(parsed_schedule) = rmac_focus::ScheduleId::parse(schedule_id) else {
            return note_card("This Focus schedule is invalid.");
        };
        let Some(schedule) = configuration
            .schedules()
            .find(|schedule| schedule.id == parsed_schedule)
        else {
            return note_card("This Focus schedule no longer exists.");
        };
        let mode_name = configuration
            .mode(&schedule.mode)
            .map(rmac_focus::Mode::name)
            .unwrap_or("Unknown Focus");
        let view = cx.entity();
        let busy = self.focus_policy_busy;
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying Focus schedule…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.focus_policy_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body = body.child(card(vec![focus_schedule_toggle_row(
            &view, schedule, mode_name, busy,
        )]));
        body = body.child(section_header("Repeat"));
        body = body.child(
            div()
                .flex()
                .gap_1()
                .mb_3()
                .children(FOCUS_DAYS.into_iter().map(|(day, label)| {
                    focus_day_button(
                        &view,
                        schedule_id,
                        day,
                        label,
                        schedule.days.contains(&day),
                        busy,
                    )
                })),
        );
        body = body.child(section_header("Time"));
        body = body.child(card(vec![
            focus_time_row(
                &view,
                schedule_id,
                "From",
                true,
                schedule.start_minute,
                busy,
            ),
            focus_time_row(&view, schedule_id, "To", false, schedule.end_minute, busy),
        ]));
        body = body.child(note_card(
            "Times use this computer’s local time. A finish time before the start time continues into the next day.",
        ));
        let remove_view = view.clone();
        let remove_schedule_id = schedule_id.to_owned();
        body.child(card(vec![ListRow::new(
            SharedString::from(format!("focus-remove-schedule-{schedule_id}")),
            div()
                .text_color(rmac_ui::mac::danger())
                .child("Delete Schedule"),
        )
        .disabled(busy)
        .on_activate(move |_, _, cx| {
            remove_view.update(cx, |settings, cx| {
                settings.remove_focus_schedule(remove_schedule_id.clone(), cx);
            });
        })
        .into_any_element()]))
    }
}
