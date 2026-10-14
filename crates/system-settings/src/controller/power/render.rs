//! System Settings Battery pane presentation, laid out like macOS 26: Low
//! Power Mode, Battery Health (its details in a sheet), then the history
//! group with its Last 24 Hours / Last 10 Days switch, "Last charged to",
//! the Battery Level chart and the Screen On Usage chart. The toolbar
//! carries the "Battery Level" subtitle.

use super::*;
use rmac_power::low_power::LowPowerMode;

const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Settings {
    pub(in crate::controller) fn render_battery(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if self.power_loading {
            cards.push(note_card("Loading battery state from the system…"));
            return self.pane(cards);
        }
        // A background battery watcher hiccup (UPower dropping off the
        // bus) degrades quietly here rather than as the window-wide
        // Settings banner (`global_settings_error`), and clears itself the
        // moment the watcher recovers.
        if let Some(detail) = &self.power_stream_error {
            cards.push(note_card(detail.clone()));
        }

        // Low Power Mode, carried out through the system power profile.
        // Windows has no Lulo session to follow the power source, so it
        // offers Never and Always.
        let profiles = &self.power.profiles;
        if profiles.available
            && profiles
                .supported
                .contains(&rmac_power::PowerProfile::PowerSaver)
        {
            let choices: Vec<PopupChoice> = LowPowerMode::ALL
                .into_iter()
                .filter(|mode| cfg!(not(windows)) || !mode.follows_power_source())
                .map(|mode| {
                    let selected = self.low_power_mode == mode;
                    let mode_view = view.clone();
                    choice(mode.label(), selected, move |_, cx| {
                        mode_view.update(cx, |settings, cx| settings.set_low_power_mode(mode, cx));
                    })
                })
                .collect();
            let current = popup_value(&choices, self.low_power_mode.label());
            let mut rows = vec![popup_row(
                "low-power-mode",
                "Low Power Mode",
                None,
                current,
                choices,
                !self.power_busy,
            )];
            if let Some(reason) = &profiles.performance_degraded {
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
            return self.pane(cards);
        };

        // Battery Health: the condition, with its details behind ⓘ.
        let sheet_view = view.clone();
        cards.push(card(vec![row_base()
            .child(text_block("Battery Health".into(), None))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(secondary())
                            .child(battery_condition(battery.capacity)),
                    )
                    .child(info_button(
                        "battery-health-info",
                        "Show Detail",
                        move |_, cx| {
                            sheet_view.update(cx, |settings, cx| {
                                settings.set_battery_health_sheet(true, cx)
                            });
                        },
                    )),
            )
            .into_any_element()]));

        match battery.history.availability {
            rmac_power::BatteryHistoryAvailability::Available => {
                cards.push(self.battery_history_group(battery, cx));
            }
            rmac_power::BatteryHistoryAvailability::TemporarilyUnavailable => {
                cards.push(note_card(
                    "Recent battery history could not be read from UPower. Current battery state remains authoritative.",
                ));
            }
            // Windows keeps no charge history an app can read: the history
            // group is simply absent there.
            rmac_power::BatteryHistoryAvailability::Unsupported => {
                if cfg!(not(windows)) {
                    cards.push(note_card(
                        "Recent battery history is unavailable because UPower does not provide it for this battery.",
                    ));
                }
            }
        }
        self.pane(cards)
    }

    fn battery_history_group(&self, battery: &rmac_power::Battery, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let long = self.battery_history_long;
        let now = now_seconds();
        let (points, start, bucket, buckets) = if long {
            let start = local_day_start(now).saturating_sub(9 * DAY);
            (&battery.history.long_points, start, HOUR, 240)
        } else {
            let hour = now - now % HOUR;
            (&battery.history.points, hour + HOUR - DAY, 15 * 60, 96)
        };
        let tab_view = view.clone();
        let tabs = tab_bar(
            "battery-history-range",
            &["Last 24 Hours", "Last 10 Days"],
            usize::from(long),
            Rc::new(move |index, _, cx| {
                tab_view.update(cx, |settings, cx| {
                    settings.set_battery_history_long(index == 1, cx)
                });
            }),
        );

        let mut group = div()
            .v_flex()
            .mb(px(style::GROUP_GAP))
            .gap(px(14.0))
            .p(px(style::ROW_PADDING))
            .rounded(px(style::GROUP_RADIUS))
            .bg(card_bg())
            .child(tabs);
        if let Some((level, at)) = last_charge(&battery.history.long_points)
            .filter(|_| long)
            .or_else(|| last_charge(&battery.history.points))
        {
            group = group.child(
                div()
                    .v_flex()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(label())
                            .child(format!("Last charged to {level}%")),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(secondary())
                            .child(charged_when(at, now)),
                    ),
            );
        }

        let ticks = |every: usize, label: &dyn Fn(u64) -> String| -> Vec<(usize, SharedString)> {
            (0..buckets)
                .filter(|index| index % every == 0)
                .map(|index| (index, label(start + index as u64 * bucket).into()))
                .collect()
        };
        let level_bars = level_bars(points, start, bucket, buckets);
        let latest = battery.percentage;
        let bars = level_bars
            .iter()
            .map(|bar| ChartBar {
                fraction: f32::from(bar.level.unwrap_or(0)) / 100.0,
                color: rmac_ui::mac::system_green(),
                marked: bar.charging,
            })
            .collect();
        let level_ticks = if long {
            ticks(24, &day_tick)
        } else {
            ticks(12, &hour_tick)
        };
        group = group.child(bar_chart(
            "battery-level-chart",
            "Battery Level",
            format!("Battery Level, {latest}% now").into(),
            bars,
            ["100%", "50%", "0%"],
            level_ticks,
        ));

        // Screen On Usage: minutes per hour, or hours per day.
        let samples: Vec<u64> = battery
            .history
            .long_points
            .iter()
            .chain(battery.history.points.iter())
            .map(|point| point.timestamp)
            .collect();
        let intervals = rmac_power::screen_usage::intervals(&self.screen_events, &samples, now);
        if !self.screen_events.is_empty() {
            let (usage_bucket, usage_buckets, full, axis): (u64, usize, f32, [&'static str; 3]) =
                if long {
                    (DAY, 10, 24.0 * 60.0, ["24h", "12h", "0h"])
                } else {
                    (HOUR, 24, 60.0, ["60m", "30m", "0m"])
                };
            let minutes = rmac_power::screen_usage::minutes_per_bucket(
                &intervals,
                start,
                usage_bucket,
                usage_buckets,
            );
            let total: u32 = minutes.iter().sum();
            let usage_ticks = (0..usage_buckets)
                .filter(|index| if long { index % 2 == 0 } else { index % 3 == 0 })
                .map(|index| {
                    let at = start + index as u64 * usage_bucket;
                    let text = if long { day_tick(at) } else { hour_tick(at) };
                    (index, SharedString::from(text))
                })
                .collect();
            group = group.child(bar_chart(
                "screen-on-usage-chart",
                "Screen On Usage",
                format!("Screen On Usage, {} hr {} min", total / 60, total % 60).into(),
                minutes
                    .into_iter()
                    .map(|minutes| ChartBar {
                        fraction: minutes as f32 / full,
                        color: rmac_ui::mac::system_blue(),
                        marked: false,
                    })
                    .collect(),
                axis,
                usage_ticks,
            ));
        }
        group
    }

    /// The Battery Health sheet behind the row's ⓘ: Battery Condition and
    /// Maximum Capacity with the Mac's explanations, then Optimised Battery
    /// Charging (UPower's charge limit) and Done.
    pub(in crate::controller) fn render_battery_health_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.battery_health_sheet {
            return None;
        }
        let battery = self.power.battery.as_ref()?;
        let view = cx.entity();
        let fact = |title: &'static str, value: SharedString, detail: &'static str| {
            div()
                .v_flex()
                .gap(px(4.0))
                .mb(px(16.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(rmac_ui::text_px(13.0))
                        .child(
                            div()
                                .font_weight(rmac_ui::mac::BOLD)
                                .text_color(label())
                                .child(title),
                        )
                        .child(div().text_color(secondary()).child(value)),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .text_color(secondary())
                        .child(detail),
                )
        };
        let condition = battery_condition(battery.capacity);
        let mut body = div().v_flex().child(fact(
            "Battery Condition",
            condition.into(),
            if condition == "Normal" {
                "Your battery is functioning normally."
            } else {
                "Your battery holds noticeably less charge than when it was new."
            },
        ));
        if let Some(capacity) = battery.capacity {
            body = body.child(fact(
                "Maximum Capacity",
                format!("{capacity}%").into(),
                "This is a measure of battery capacity relative to when it was new. Lower capacity may result in fewer hours of usage between charges.",
            ));
        }
        if let Some(cycles) = battery.charge_cycles {
            body = body.child(fact(
                "Cycle Count",
                cycles.to_string().into(),
                "One cycle is a full discharge's worth of use, whether in one go or over several charges.",
            ));
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
                body = body.child(
                    row_base()
                        .px(px(0.0))
                        .items_start()
                        .child(text_block(
                            "Optimised Battery Charging".into(),
                            Some(charge_threshold_description(&threshold).into()),
                        ))
                        .child(toggle),
                );
            }
            rmac_power::ChargeThresholdAvailability::MultipleBatteries => {
                body = body.child(footnote(
                    "Optimised charging is unavailable for multiple system batteries until each battery can be controlled separately.",
                ));
            }
            rmac_power::ChargeThresholdAvailability::Unsupported => {
                if cfg!(not(windows)) {
                    body = body.child(footnote(
                        "Optimised charging is unavailable because UPower reports no writable charge limit for this battery.",
                    ));
                }
            }
        }
        let done_view = view.clone();
        Some(
            settings_sheet(
                "battery-health-sheet",
                460.0,
                360.0,
                None,
                body.into_any_element(),
                vec![sheet_default_button(
                    "battery-health-done",
                    "Done",
                    move |_, cx| {
                        done_view.update(cx, |settings, cx| {
                            settings.set_battery_health_sheet(false, cx)
                        });
                    },
                )],
            )
            .into_any_element(),
        )
    }

    /// The Mac's two-line Battery title: "Battery Level: 100%" under it.
    pub(in crate::controller) fn battery_toolbar_subtitle(&self) -> Option<SharedString> {
        self.power
            .battery
            .as_ref()
            .map(|battery| format!("Battery Level: {}%", battery.percentage).into())
    }
}

/// The Mac's Battery Condition: Normal, or Service Recommended below 80 %.
fn battery_condition(capacity: Option<u8>) -> &'static str {
    match capacity {
        Some(capacity) if capacity < 80 => "Service Recommended",
        Some(_) => "Normal",
        None => "Unknown",
    }
}
