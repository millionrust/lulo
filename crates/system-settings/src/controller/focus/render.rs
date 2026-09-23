//! Focus settings presentation.

use super::*;

mod mode;
mod schedule;

impl Settings {
    pub(in crate::controller) fn render_focus(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        let refresh_view = view.clone();
        cards.push(
            div().flex().justify_end().mb_2().child(
                Button::new("focus-refresh", "Refresh")
                    .ghost()
                    .busy(self.focus_policy_loading || self.focus_policy_busy)
                    .disabled(self.focus_policy_loading || self.focus_policy_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_focus(cx));
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
                        Button::new("focus-turn-off", "Turn Off")
                            .ghost()
                            .disabled(self.focus_policy_loading || self.focus_policy_busy)
                            .on_click(move |_, _, cx| {
                                turn_off_view.update(cx, |settings, cx| settings.disable_focus(cx));
                            }),
                    );
                }
                FocusCurrentAction::EditSchedule(schedule_id) => {
                    let edit_view = view.clone();
                    let schedule_id = schedule_id.as_str().to_owned();
                    current = current.child(
                        Button::new("focus-edit-active-schedule", "Edit Schedule")
                            .ghost()
                            .disabled(self.focus_policy_loading || self.focus_policy_busy)
                            .on_click(move |_, _, cx| {
                                edit_view.update(cx, |settings, cx| {
                                    settings.push(
                                        SubPage::FocusSchedule {
                                            schedule_id: schedule_id.clone(),
                                        },
                                        cx,
                                    );
                                });
                            }),
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
                        .text_size(rmac_ui::text_px(13.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(style::heading_text())
                        .mt(px(style::SECTION_TOP))
                        .mb(px(style::SECTION_BOTTOM))
                        .px(px(style::ROW_PADDING))
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
}
