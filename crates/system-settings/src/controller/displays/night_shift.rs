//! Displays ▸ Night Shift…: the Mac's sheet (Schedule, Turn on until
//! tomorrow, Colour temperature), carried out by `rmac-night-shift.service`
//! through `wlsunset` (`rmac_shortcuts::night_shift`).

use super::*;
use rmac_shortcuts::night_shift::{Policy, Schedule};

/// Half-hour steps for the From / To pop-ups.
const STEPS: u16 = 48;

fn time_label(minutes: u16) -> String {
    let hour = minutes / 60 % 24;
    let minute = minutes % 60;
    let (display, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{display}:{minute:02} {suffix}")
}

/// The next local 07:00 after now, as the Mac's "until tomorrow" ends in
/// the morning.
fn next_morning() -> u64 {
    use chrono::{Duration, Local, TimeZone as _};
    let now = Local::now();
    let today = now.date_naive().and_hms_opt(7, 0, 0);
    let morning = today.and_then(|today| Local.from_local_datetime(&today).earliest());
    match morning {
        Some(morning) if morning > now => morning.timestamp().max(0) as u64,
        Some(morning) => (morning + Duration::days(1)).timestamp().max(0) as u64,
        None => now.timestamp().max(0) as u64 + 12 * 3600,
    }
}

impl Settings {
    pub(in crate::controller) fn open_night_shift(&mut self, cx: &mut Context<Self>) {
        self.night_shift_sheet = true;
        self.night_shift_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (policy, available) = cx
                .background_executor()
                .spawn(async {
                    let policy = rmac_shortcuts::night_shift::policy_path()
                        .map(|path| rmac_shortcuts::night_shift::read_policy(&path))
                        .unwrap_or_default();
                    (policy, rmac_shortcuts::night_shift::available())
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.night_shift_policy = policy;
                this.night_shift_available = available;
                this.night_shift_slider = Self::night_shift_slider(cx, f32::from(policy.warmth));
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn close_night_shift(&mut self, cx: &mut Context<Self>) {
        self.night_shift_sheet = false;
        cx.notify();
    }

    pub(in crate::controller) fn night_shift_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                let mut policy = this.night_shift_policy;
                policy.warmth = value.start().round().clamp(0.0, 100.0) as u8;
                this.change_night_shift(policy, cx);
            }
        })
        .detach();
        slider
    }

    /// Save `policy` and restart the service, off the UI thread. Slider
    /// drags coalesce: only the latest change is written.
    pub(in crate::controller) fn change_night_shift(
        &mut self,
        policy: Policy,
        cx: &mut Context<Self>,
    ) {
        if policy == self.night_shift_policy {
            return;
        }
        self.night_shift_policy = policy;
        self.night_shift_generation = self.night_shift_generation.wrapping_add(1);
        let generation = self.night_shift_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let current = this
                .update(cx, |this: &mut Settings, _| {
                    this.night_shift_generation == generation
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            // systemctl is a child process: keep it off GPUI's executor.
            let result =
                blocking::unblock(move || rmac_shortcuts::night_shift::apply(&policy)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.night_shift_error = result
                    .err()
                    .map(|error| format!("Night Shift could not be changed: {error}").into());
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn render_night_shift_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.night_shift_sheet {
            return None;
        }
        let view = cx.entity();
        let policy = self.night_shift_policy;
        let enabled = self.night_shift_available;
        let (from, to) = match policy.schedule {
            Schedule::Custom { from, to } => (from, to),
            Schedule::Off => (22 * 60, 7 * 60),
        };
        let schedule_choices = vec![
            {
                let view = view.clone();
                choice("Off", policy.schedule == Schedule::Off, move |_, cx| {
                    view.update(cx, |settings, cx| {
                        let mut policy = settings.night_shift_policy;
                        policy.schedule = Schedule::Off;
                        settings.change_night_shift(policy, cx);
                    });
                })
            },
            {
                let view = view.clone();
                choice("Custom", policy.schedule != Schedule::Off, move |_, cx| {
                    view.update(cx, |settings, cx| {
                        let mut policy = settings.night_shift_policy;
                        policy.schedule = Schedule::Custom { from, to };
                        settings.change_night_shift(policy, cx);
                    });
                })
            },
        ];
        let time_choices = |is_from: bool| -> Vec<PopupChoice> {
            (0..STEPS)
                .map(|step| {
                    let minutes = step * 30;
                    let selected = if is_from {
                        from == minutes
                    } else {
                        to == minutes
                    };
                    let view = view.clone();
                    choice(time_label(minutes), selected, move |_, cx| {
                        view.update(cx, |settings, cx| {
                            let mut policy = settings.night_shift_policy;
                            let (from, to) = match policy.schedule {
                                Schedule::Custom { from, to } => (from, to),
                                Schedule::Off => (22 * 60, 7 * 60),
                            };
                            policy.schedule = if is_from {
                                Schedule::Custom { from: minutes, to }
                            } else {
                                Schedule::Custom { from, to: minutes }
                            };
                            settings.change_night_shift(policy, cx);
                        });
                    })
                })
                .collect()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let manual_view = view.clone();
        let manual = Toggle::new("night-shift-manual")
            .checked(policy.manual_active(now))
            .disabled(!enabled)
            .on_click(move |on, _, cx| {
                let on = *on;
                manual_view.update(cx, |settings, cx| {
                    let mut policy = settings.night_shift_policy;
                    policy.manual_until = on.then(next_morning);
                    settings.change_night_shift(policy, cx);
                });
            });
        let mut rows = vec![popup_row(
            "night-shift-schedule",
            "Schedule",
            None,
            if policy.schedule == Schedule::Off {
                "Off".into()
            } else {
                "Custom".into()
            },
            schedule_choices,
            enabled,
        )];
        if policy.schedule != Schedule::Off {
            rows.push(popup_row(
                "night-shift-from",
                "From",
                None,
                time_label(from).into(),
                time_choices(true),
                enabled,
            ));
            rows.push(popup_row(
                "night-shift-to",
                "To",
                None,
                time_label(to).into(),
                time_choices(false),
                enabled,
            ));
        }
        rows.push(
            row_base()
                .child(text_block("Turn on until tomorrow".into(), None))
                .child(manual)
                .into_any_element(),
        );
        rows.push(
            row_base()
                .child(text_block("Colour temperature".into(), None))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(secondary())
                        .child("Less Warm")
                        .child(
                            div().w(px(style::SLIDER_WIDTH * 0.7)).child(
                                Slider::new(&self.night_shift_slider)
                                    .accessible_name("Colour temperature")
                                    .disabled(!enabled)
                                    .w_full(),
                            ),
                        )
                        .child("More Warm"),
                )
                .into_any_element(),
        );
        let mut body = div()
            .v_flex()
            .child(
                div()
                    .mb(px(12.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary())
                    .child("Night Shift automatically shifts the colours of your display to the warmer end of the colour spectrum after dark. This may help you get a better night’s sleep."),
            )
            .child(card(rows));
        if !enabled {
            body = body.child(footnote(
                "Night Shift needs the wlsunset package. Install it, then open this sheet again.",
            ));
        }
        if let Some(error) = &self.night_shift_error {
            body = body.child(note_card(error.clone()));
        }
        let done_view = view.clone();
        Some(
            settings_sheet(
                "night-shift-sheet",
                500.0,
                if policy.schedule == Schedule::Off {
                    330.0
                } else {
                    410.0
                },
                None,
                body.into_any_element(),
                vec![sheet_default_button(
                    "night-shift-done",
                    "Done",
                    move |_, cx| {
                        done_view.update(cx, |settings, cx| settings.close_night_shift(cx));
                    },
                )],
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::time_label;

    #[test]
    fn times_read_like_the_mac() {
        assert_eq!(time_label(0), "12:00 AM");
        assert_eq!(time_label(22 * 60), "10:00 PM");
        assert_eq!(time_label(7 * 60 + 30), "7:30 AM");
        assert_eq!(time_label(12 * 60), "12:00 PM");
    }
}
