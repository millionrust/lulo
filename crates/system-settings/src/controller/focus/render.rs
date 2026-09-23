//! Focus settings presentation: the Mac's list of Focus modes as 52 pt icon
//! rows (design-lab/settings.html, "Focus").

use super::*;

mod mode;
mod schedule;

impl Settings {
    pub(in crate::controller) fn render_focus(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let busy = self.focus_policy_loading || self.focus_policy_busy;
        let mut cards = Vec::new();
        if busy {
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
        let mut footer = Vec::new();
        if let Some(state) = &self.focus_policy_state {
            match focus_current_action(state.source.as_ref()) {
                FocusCurrentAction::TurnOffManual => {
                    let turn_off_view = view.clone();
                    footer.push(
                        push_button("focus-turn-off", "Turn Off Focus")
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                turn_off_view.update(cx, |settings, cx| settings.disable_focus(cx));
                            })
                            .into_any_element(),
                    );
                }
                FocusCurrentAction::EditSchedule(schedule_id) => {
                    let edit_view = view.clone();
                    let schedule_id = schedule_id.as_str().to_owned();
                    footer.push(
                        push_button("focus-edit-active-schedule", "Edit Schedule…")
                            .disabled(busy)
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
                            .into_any_element(),
                    );
                }
                FocusCurrentAction::None => {}
            }
        }
        if let Some(configuration) = &self.focus_policy_config {
            let state = self.focus_policy_state.as_ref();
            let active_mode = state
                .filter(|state| state.projection.enabled)
                .and_then(|state| state.mode_id.as_deref());
            let scheduled = matches!(
                state.and_then(|state| state.source.as_ref()),
                Some(rmac_focus::ActivationSource::Schedule(_))
            );
            let mode_rows = configuration
                .modes()
                .map(|mode| {
                    let mode_id = mode.id().as_str().to_owned();
                    let active = active_mode == Some(mode.id().as_str());
                    let target_view = view.clone();
                    large_nav_row(
                        SharedString::from(format!("focus-mode-{mode_id}")),
                        tile26("icons/moon.svg", hsl(0x5e5ce6)),
                        mode.name().to_owned(),
                        (active && scheduled).then(|| subtitle_text("Scheduled")),
                        active.then(|| "On".into()),
                        move |_, cx| {
                            let mode_id = mode_id.clone();
                            target_view.update(cx, |settings, cx| {
                                settings.push(SubPage::FocusMode { mode_id }, cx)
                            });
                        },
                    )
                })
                .collect();
            cards.push(card(mode_rows));
        }
        let refresh_view = view.clone();
        footer.push(
            push_button("focus-refresh", "Refresh")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_focus(cx));
                })
                .into_any_element(),
        );
        cards.push(footer_buttons(footer));
        self.pane(cards)
    }
}
