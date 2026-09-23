//! System Settings Battery pane presentation, laid out like macOS 26
//! (design-lab/settings.html): the energy-mode pop-up, Battery Health, the
//! charge history, then charging options. The toolbar carries the "Battery
//! Level" subtitle.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_battery(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let footer = footer_buttons(vec![push_button("power-refresh", "Refresh")
            .busy(self.power_busy)
            .disabled(self.power_loading || self.power_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_power(cx));
            })
            .into_any_element()]);
        let mut cards = Vec::new();
        if self.power_loading {
            cards.push(note_card("Loading battery state from the system…"));
            return self.pane(cards);
        }

        // Energy mode (the Mac's "Low Power Mode" pop-up) from
        // power-profiles-daemon.
        if self.power.profiles.available && !self.power.profiles.supported.is_empty() {
            let choices: Vec<PopupChoice> = self
                .power
                .profiles
                .supported
                .iter()
                .map(|profile| {
                    let profile = *profile;
                    let selected = self.power.profiles.active == Some(profile);
                    let profile_view = view.clone();
                    choice(profile.label(), selected, move |_, cx| {
                        if !selected {
                            profile_view
                                .update(cx, |settings, cx| settings.set_power_profile(profile, cx));
                        }
                    })
                })
                .collect();
            let current = popup_value(&choices, "Unknown");
            let mut rows = vec![popup_row(
                "power-profile",
                "Energy Mode",
                None,
                current,
                choices,
                !self.power_busy,
            )];
            if let Some(reason) = &self.power.profiles.performance_degraded {
                rows.push(
                    row_base()
                        .child(text_block("High Power Limited".into(), None))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(secondary())
                                .child(power_degradation_label(reason)),
                        )
                        .into_any_element(),
                );
            }
            cards.push(card(rows));
        }

        let Some(battery) = &self.power.battery else {
            cards.push(note_card(
                "No system battery was detected. This computer is using external power.",
            ));
            cards.push(footer);
            return self.pane(cards);
        };

        let value = |title: &'static str, value: SharedString| {
            row_base()
                .child(text_block(title.into(), None))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(value),
                )
                .into_any_element()
        };
        let condition = battery.capacity.map_or("Unknown", |capacity| {
            if capacity < 75 {
                "Service Recommended"
            } else {
                "Normal"
            }
        });
        let mut health_rows = vec![value("Battery Health", condition.into())];
        if let Some(capacity) = battery.capacity {
            health_rows.push(value("Maximum Capacity", format!("{capacity}%").into()));
        }
        if let Some(cycles) = battery.charge_cycles {
            health_rows.push(value("Cycle Count", cycles.to_string().into()));
        }
        cards.push(card(health_rows));

        let mut status_rows = vec![
            value("Status", battery.state.label().into()),
            value(
                "Power Source",
                if battery.on_battery {
                    "Battery".into()
                } else {
                    "Power Adapter".into()
                },
            ),
        ];
        if let Some(seconds) = battery.seconds_remaining {
            status_rows.push(value(
                if matches!(
                    battery.state,
                    rmac_power::BatteryState::Charging | rmac_power::BatteryState::PendingCharge
                ) {
                    "Time to Full"
                } else {
                    "Time Remaining"
                },
                format_power_duration(seconds).into(),
            ));
        }
        if let Some(rate) = battery.energy_rate_watts {
            status_rows.push(value("Energy Rate", format!("{rate:.1} W").into()));
        }
        if let Some(model) = &battery.model {
            status_rows.push(value("Battery", model.clone().into()));
        }
        cards.push(card(status_rows));

        match battery.history.availability {
            rmac_power::BatteryHistoryAvailability::Available
                if battery.history.points.is_empty() =>
            {
                cards.push(note_card(
                    "UPower supports battery history, but it has not recorded any charge samples in the last 24 hours.",
                ));
            }
            rmac_power::BatteryHistoryAvailability::Available => {
                cards.push(battery_history_card(&battery.history.points));
            }
            rmac_power::BatteryHistoryAvailability::TemporarilyUnavailable => {
                cards.push(note_card(
                    "Recent battery history could not be read from UPower. Current battery state remains authoritative.",
                ));
            }
            rmac_power::BatteryHistoryAvailability::Unsupported => {
                cards.push(note_card(
                    "Recent battery history is unavailable because UPower does not provide it for this battery.",
                ));
            }
        }

        match battery.charge_threshold.availability {
            rmac_power::ChargeThresholdAvailability::Available => {
                let threshold = battery.charge_threshold.clone();
                let threshold_view = view.clone();
                let threshold_for_action = threshold.clone();
                let toggle = Toggle::new("battery-charge-threshold")
                    .checked(threshold.enabled)
                    .disabled(self.power_busy || !threshold.can_change())
                    .on_click(move |enabled, _, cx| {
                        threshold_view.update(cx, |settings, cx| {
                            settings.set_charge_threshold(
                                threshold_for_action.clone(),
                                *enabled,
                                cx,
                            );
                        });
                    });
                cards.push(card(vec![row_base()
                    .items_start()
                    .child(text_block(
                        "Optimised Battery Charging".into(),
                        Some(charge_threshold_description(&threshold).into()),
                    ))
                    .child(toggle)
                    .into_any_element()]));
            }
            rmac_power::ChargeThresholdAvailability::MultipleBatteries => {
                cards.push(footnote(
                    "Optimised charging is unavailable for multiple system batteries until each battery can be controlled separately.",
                ));
            }
            rmac_power::ChargeThresholdAvailability::Unsupported => {
                cards.push(footnote(
                    "Optimised charging is unavailable because UPower reports no writable charge limit for this battery.",
                ));
            }
        }
        cards.push(footer);
        self.pane(cards)
    }

    /// The Mac's two-line Battery title: "Battery Level: 100%" under it.
    pub(in crate::controller) fn battery_toolbar_subtitle(&self) -> Option<SharedString> {
        self.power
            .battery
            .as_ref()
            .map(|battery| format!("Battery Level: {}%", battery.percentage).into())
    }
}
