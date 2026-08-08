//! System Settings Battery and power-profile pane presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_battery(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("Battery & Energy"),
            )
            .child(
                Button::new("power-refresh", "Refresh")
                    .ghost()
                    .busy(self.power_busy)
                    .disabled(self.power_loading || self.power_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_power(cx));
                    }),
            )];
        if self.power_loading {
            cards.push(note_card("Loading battery state from the system…"));
            return self.pane(cards);
        }

        if let Some(battery) = &self.power.battery {
            let mut status_rows = vec![
                value_row(
                    "icons/battery-charging.svg",
                    hsl(0x34c759),
                    "Charge".into(),
                    format!("{}%", battery.percentage).into(),
                ),
                value_row(
                    "icons/info.svg",
                    secondary(),
                    "Status".into(),
                    battery.state.label().into(),
                ),
                value_row(
                    "icons/power.svg",
                    secondary(),
                    "Power Source".into(),
                    if battery.on_battery {
                        "Battery".into()
                    } else {
                        "Power Adapter".into()
                    },
                ),
            ];
            if let Some(seconds) = battery.seconds_remaining {
                status_rows.push(value_row(
                    "icons/clock.svg",
                    secondary(),
                    if matches!(
                        battery.state,
                        rmac_power::BatteryState::Charging
                            | rmac_power::BatteryState::PendingCharge
                    ) {
                        "Time to Full".into()
                    } else {
                        "Time Remaining".into()
                    },
                    format_power_duration(seconds).into(),
                ));
            }
            if let Some(rate) = battery.energy_rate_watts {
                status_rows.push(value_row(
                    "icons/power.svg",
                    secondary(),
                    "Energy Rate".into(),
                    format!("{rate:.1} W").into(),
                ));
            }
            if let Some(model) = &battery.model {
                status_rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Battery".into(),
                    model.clone().into(),
                ));
            }
            cards.push(card(status_rows));

            let condition = battery.capacity.map_or("Unknown", |capacity| {
                if capacity < 75 {
                    "Service Recommended"
                } else {
                    "Normal"
                }
            });
            let mut health_rows = vec![value_row(
                "icons/heart-handshake.svg",
                hsl(0x34c759),
                "Condition".into(),
                condition.into(),
            )];
            if let Some(capacity) = battery.capacity {
                health_rows.insert(
                    0,
                    value_row(
                        "icons/battery-charging.svg",
                        hsl(0x34c759),
                        "Maximum Capacity".into(),
                        format!("{capacity}%").into(),
                    ),
                );
            }
            if let Some(cycles) = battery.charge_cycles {
                health_rows.push(value_row(
                    "icons/history.svg",
                    secondary(),
                    "Cycle Count".into(),
                    cycles.to_string().into(),
                ));
            }
            cards.push(section_header("Battery Health"));
            cards.push(card(health_rows));

            cards.push(section_header("Charging"));
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
                        .child(tile("icons/battery-charging.svg", hsl(0x34c759), 22.0))
                        .child(text_block(
                            "Optimized Charging".into(),
                            Some(charge_threshold_description(&threshold).into()),
                        ))
                        .child(toggle)
                        .into_any_element()]));
                }
                rmac_power::ChargeThresholdAvailability::MultipleBatteries => {
                    cards.push(note_card(
                        "Optimized charging is unavailable for multiple system batteries until each battery can be controlled separately.",
                    ));
                }
                rmac_power::ChargeThresholdAvailability::Unsupported => {
                    cards.push(note_card(
                        "Optimized charging is unavailable because UPower reports no writable charge limit for this battery.",
                    ));
                }
            }

            cards.push(section_header("Battery Level"));
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
        } else {
            cards.push(note_card(
                "No system battery was detected. This computer is using external power.",
            ));
        }

        if self.power.profiles.available && !self.power.profiles.supported.is_empty() {
            cards.push(section_header("Energy Mode"));
            let rows = self
                .power
                .profiles
                .supported
                .iter()
                .map(|profile| {
                    let profile = *profile;
                    let selected = self.power.profiles.active == Some(profile);
                    let profile_view = view.clone();
                    power_profile_row(profile, selected, self.power_busy)
                        .on_activate(move |_, _, cx| {
                            if !selected {
                                profile_view.update(cx, |settings, cx| {
                                    settings.set_power_profile(profile, cx)
                                });
                            }
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
            if let Some(reason) = &self.power.profiles.performance_degraded {
                cards.push(card(vec![value_row(
                    "icons/info.svg",
                    hsl(0xff9500),
                    "High Power Limited".into(),
                    power_degradation_label(reason).into(),
                )]));
            }
        } else {
            cards.push(note_card(
                "Power profile selection is unavailable on this computer.",
            ));
        }
        self.pane(cards)
    }
}
