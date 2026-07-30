//! Shared typed row, control, icon, and visualization builders.

use super::*;
// ---- row / control builders ----------------------------------------------

pub(super) fn network_ip_method_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    selected: &rmac_network::IpMethod,
    enabled: bool,
) -> AnyElement {
    let options = match family {
        rmac_network::IpFamily::V4 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
        rmac_network::IpFamily::V6 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("DHCP", rmac_network::IpMethod::Dhcp),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
    };
    let mut control = div().flex().gap_1().w(px(390.0));
    for (index, (label, method)) in options.into_iter().enumerate() {
        let method_view = view.clone();
        let chosen = method.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!(
                    "network-{}-method-{index}",
                    match family {
                        rmac_network::IpFamily::V4 => "ipv4",
                        rmac_network::IpFamily::V6 => "ipv6",
                    }
                ))),
                label,
            )
            .flex_1()
            .selected(*selected == method)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                method_view.update(cx, |settings, cx| {
                    settings.set_network_ip_method(family, chosen.clone(), window, cx)
                });
            }),
        );
    }
    row_base()
        .child(text_block(
            "Configure".into(),
            Some("Choose how this connection receives addresses".into()),
        ))
        .child(control)
        .into_any_element()
}

pub(super) fn network_field_row(
    title: &'static str,
    subtitle: &'static str,
    editor: &Entity<InputState>,
    enabled: bool,
) -> AnyElement {
    row_base()
        .child(text_block(title.into(), Some(subtitle.into())))
        .child(
            div()
                .w(px(390.0))
                .child(TextField::new(editor).small().disabled(!enabled)),
        )
        .into_any_element()
}

pub(super) fn network_dns_policy_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let id = match family {
        rmac_network::IpFamily::V4 => "network-ipv4-ignore-auto-dns",
        rmac_network::IpFamily::V6 => "network-ipv6-ignore-auto-dns",
    };
    row_base()
        .child(text_block(
            "Use only these DNS servers".into(),
            Some("Ignore DNS supplied automatically by the network".into()),
        ))
        .child(
            Toggle::new(id)
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_ignore_auto_dns(family, *value, cx)
                    });
                }),
        )
        .into_any_element()
}

pub(super) fn network_proxy_method_row(
    view: Entity<Settings>,
    selected: rmac_network::ProxyMethod,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(390.0));
    for (index, (label, method)) in [
        ("Off", rmac_network::ProxyMethod::None),
        ("Automatic", rmac_network::ProxyMethod::Automatic),
    ]
    .into_iter()
    .enumerate()
    {
        let method_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("network-proxy-method-{index}"))),
                label,
            )
            .flex_1()
            .selected(selected == method)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                method_view.update(cx, |settings, cx| {
                    settings.set_network_proxy_method(method, window, cx)
                });
            }),
        );
    }
    row_base()
        .child(text_block(
            "Configure".into(),
            Some("Use a proxy auto-configuration source".into()),
        ))
        .child(control)
        .into_any_element()
}

pub(super) fn network_proxy_browser_row(
    view: Entity<Settings>,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    row_base()
        .child(text_block(
            "Web browsers only".into(),
            Some("Other applications may ignore this proxy configuration".into()),
        ))
        .child(
            Toggle::new("network-proxy-browser-only")
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_proxy_browser_only(*value, cx)
                    });
                }),
        )
        .into_any_element()
}

pub(super) fn row_base() -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(44.0))
        .px_3()
        .py_2()
}

pub(super) fn text_block(title: SharedString, sub: Option<SharedString>) -> Div {
    let mut b = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(s) = sub {
        b = b.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary())
                .child(s),
        );
    }
    b
}

/// A plain card-section label row (no control).
pub(super) fn label_row(title: &'static str, value: Option<SharedString>) -> Div {
    let mut r = row_base().child(
        div()
            .flex_1()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(v) = value {
        r = r.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
    }
    r
}

/// A read-only row with a right-aligned value.
pub(super) fn value_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: SharedString,
) -> AnyElement {
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

pub(super) fn locale_format(snapshot: &rmac_locale::Snapshot, key: &str) -> String {
    snapshot.effective_format_locale(key).to_owned()
}

pub(super) fn locale_preview_row(
    icon: &'static str,
    title: &'static str,
    source: String,
    example: Option<&str>,
) -> AnyElement {
    row_base()
        .child(tile(icon, secondary(), 22.0))
        .child(text_block(
            title.into(),
            Some(format!("Locale: {source}").into()),
        ))
        .child(
            div()
                .max_w(px(260.0))
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(example.unwrap_or("Uses locale convention").to_owned()),
        )
        .into_any_element()
}

/// An informational note card, e.g. to flag a pane as simulated/demo state
/// rather than a reflection of (or control over) real system hardware.
pub(super) fn note_card(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mb_3()
        .px_3()
        .py_2p5()
        .rounded(px(10.0))
        .bg(rmac_ui::mac::warning_background())
        .border_1()
        .border_color(rmac_ui::mac::warning_border())
        .child(glyph("icons/info.svg", 15.0, rmac_ui::mac::warning_text()))
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
        )
}

/// A section header above a card (gray small caps-ish title).
pub(super) fn section_header(title: impl Into<SharedString>) -> Div {
    div()
        .px_1()
        .pt_2()
        .pb_1()
        .text_size(rmac_ui::text_px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .text_color(secondary())
        .child(title.into())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn notification_toggle_row(
    view: &Entity<Settings>,
    app_id: &str,
    id: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    disabled: bool,
    change: fn(bool) -> NotificationPolicyChange,
) -> AnyElement {
    let app = app_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "notification-{id}-{app_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |value, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.apply_notification_policy(app.clone(), change(*value), cx);
        });
    });
    row_base()
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(toggle)
        .into_any_element()
}

pub(super) fn application_icon(
    icon: Option<&PathBuf>,
    fallback_icon: &'static str,
    fallback_color: Hsla,
) -> AnyElement {
    match icon {
        Some(icon) => img(icon.clone())
            .w(px(22.0))
            .h(px(22.0))
            .flex_none()
            .into_any_element(),
        None => tile(fallback_icon, fallback_color, 22.0).into_any_element(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn application_nav_row(
    view: &Entity<Settings>,
    app_id: &str,
    display_name: &str,
    icon: Option<&PathBuf>,
    status: &'static str,
    enabled: bool,
    target: SubPage,
) -> AnyElement {
    let target_view = view.clone();
    row_base()
        .id(SharedString::from(format!("notification-app-{app_id}")))
        .cursor_pointer()
        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
        .child(application_icon(
            icon,
            "icons/bell.svg",
            if enabled { accent() } else { secondary() },
        ))
        .child(text_block(display_name.to_owned().into(), None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(status),
        )
        .child(glyph(
            "icons/chevron-right.svg",
            14.0,
            rmac_ui::mac::text_tertiary(),
        ))
        .on_click(move |_, _, cx| {
            let target = target.clone();
            target_view.update(cx, |settings, cx| settings.push(target, cx));
        })
        .into_any_element()
}

pub(super) fn focus_schedule_row(
    view: &Entity<Settings>,
    schedule: &rmac_focus::Schedule,
    mode_name: &str,
    disabled: bool,
) -> AnyElement {
    let schedule_id = schedule.id.as_str().to_owned();
    let toggle_schedule_id = schedule_id.clone();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-schedule-{schedule_id}"
    ))))
    .checked(schedule.enabled)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_schedule_enabled(toggle_schedule_id.clone(), *enabled, cx);
        });
    });
    let edit_view = view.clone();
    row_base()
        .child(tile("icons/clock.svg", accent(), 22.0))
        .child(text_block(
            mode_name.to_owned().into(),
            Some(focus_schedule_summary(schedule).into()),
        ))
        .child(
            div()
                .id(SharedString::from(format!(
                    "focus-edit-schedule-{schedule_id}"
                )))
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .text_size(rmac_ui::text_px(12.0))
                .text_color(accent())
                .when(!disabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .on_click(move |_, _, cx| {
                            let schedule_id = schedule_id.clone();
                            edit_view.update(cx, |settings, cx| {
                                settings.push(SubPage::FocusSchedule { schedule_id }, cx);
                            });
                        })
                })
                .child("Edit"),
        )
        .child(toggle)
        .into_any_element()
}

pub(super) fn focus_schedule_toggle_row(
    view: &Entity<Settings>,
    schedule: &rmac_focus::Schedule,
    mode_name: &str,
    disabled: bool,
) -> AnyElement {
    let schedule_id = schedule.id.as_str().to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-schedule-enabled-{schedule_id}"
    ))))
    .checked(schedule.enabled)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_schedule_enabled(schedule_id.clone(), *enabled, cx);
        });
    });
    row_base()
        .child(tile("icons/moon.svg", accent(), 22.0))
        .child(text_block(
            mode_name.to_owned().into(),
            Some("Turn this schedule on automatically".into()),
        ))
        .child(toggle)
        .into_any_element()
}

pub(super) fn focus_day_button(
    view: &Entity<Settings>,
    schedule_id: &str,
    day: rmac_focus::Weekday,
    day_label: &'static str,
    selected: bool,
    disabled: bool,
) -> Stateful<Div> {
    let control_view = view.clone();
    let schedule_id = schedule_id.to_owned();
    div()
        .id(SharedString::from(format!(
            "focus-day-{schedule_id}-{day:?}"
        )))
        .flex_1()
        .h(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(16.0))
        .text_size(rmac_ui::text_px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .bg(if selected { accent() } else { card_bg() })
        .text_color(if selected { on_accent() } else { label() })
        .border_1()
        .border_color(if selected { accent() } else { sep() })
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(|hover| hover.opacity(0.82))
                .on_click(move |_, _, cx| {
                    control_view.update(cx, |settings, cx| {
                        settings.set_focus_schedule_day(schedule_id.clone(), day, !selected, cx);
                    });
                })
        })
        .child(day_label)
}

pub(super) fn focus_time_row(
    view: &Entity<Settings>,
    schedule_id: &str,
    title: &'static str,
    start: bool,
    minute: u16,
    disabled: bool,
) -> AnyElement {
    let previous = (minute + 1_440 - 15) % 1_440;
    let next = (minute + 15) % 1_440;
    let previous_view = view.clone();
    let next_view = view.clone();
    let previous_schedule = schedule_id.to_owned();
    let next_schedule = schedule_id.to_owned();
    let decrement = div()
        .id(SharedString::from(format!(
            "focus-time-minus-{schedule_id}-{start}"
        )))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rmac_ui::mac::hover())
        .text_size(rmac_ui::text_px(18.0))
        .when(!disabled, |button| {
            button.cursor_pointer().on_click(move |_, _, cx| {
                previous_view.update(cx, |settings, cx| {
                    settings.set_focus_schedule_time(
                        previous_schedule.clone(),
                        start,
                        previous,
                        cx,
                    );
                });
            })
        })
        .child("−");
    let increment = div()
        .id(SharedString::from(format!(
            "focus-time-plus-{schedule_id}-{start}"
        )))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rmac_ui::mac::hover())
        .text_size(rmac_ui::text_px(18.0))
        .when(!disabled, |button| {
            button.cursor_pointer().on_click(move |_, _, cx| {
                next_view.update(cx, |settings, cx| {
                    settings.set_focus_schedule_time(next_schedule.clone(), start, next, cx);
                });
            })
        })
        .child("+");
    row_base()
        .child(text_block(title.into(), None))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(decrement)
                .child(
                    div()
                        .w(px(74.0))
                        .text_center()
                        .text_size(rmac_ui::text_px(12.0))
                        .child(focus_time(minute)),
                )
                .child(increment),
        )
        .into_any_element()
}

pub(super) fn focus_urgent_row(
    view: &Entity<Settings>,
    mode_id: &str,
    checked: bool,
    disabled: bool,
) -> AnyElement {
    let mode_id = mode_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-urgent-{mode_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_urgent(mode_id.clone(), *enabled, cx);
        });
    });
    row_base()
        .child(text_block(
            "Allow urgent notifications".into(),
            Some("Let urgent notifications through while this Focus is on".into()),
        ))
        .child(toggle)
        .into_any_element()
}

pub(super) fn focus_allowed_app_row(
    view: &Entity<Settings>,
    mode_id: &str,
    app_id: &str,
    display_name: &str,
    icon: Option<&PathBuf>,
    checked: bool,
    disabled: bool,
) -> AnyElement {
    let mode_id = mode_id.to_owned();
    let application_id = app_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-allowed-{mode_id}-{app_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |allowed, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_allowed_app(mode_id.clone(), application_id.clone(), *allowed, cx);
        });
    });
    row_base()
        .child(application_icon(icon, "icons/app-window.svg", secondary()))
        .child(text_block(display_name.to_owned().into(), None))
        .child(toggle)
        .into_any_element()
}

pub(super) fn focus_schedule_summary(schedule: &rmac_focus::Schedule) -> String {
    let days = schedule
        .days
        .iter()
        .map(|day| match day {
            rmac_focus::Weekday::Monday => "Mon",
            rmac_focus::Weekday::Tuesday => "Tue",
            rmac_focus::Weekday::Wednesday => "Wed",
            rmac_focus::Weekday::Thursday => "Thu",
            rmac_focus::Weekday::Friday => "Fri",
            rmac_focus::Weekday::Saturday => "Sat",
            rmac_focus::Weekday::Sunday => "Sun",
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{days} · {}–{}",
        focus_time(schedule.start_minute),
        focus_time(schedule.end_minute)
    )
}

pub(super) fn focus_time(minute: u16) -> String {
    let hour = minute / 60;
    let minute = minute % 60;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    format!("{hour}:{minute:02} {suffix}")
}

pub(super) fn bluetooth_device_row(
    view: &Entity<Settings>,
    device: &rmac_bluetooth::Device,
    busy: bool,
    forgetting: bool,
) -> AnyElement {
    let mut details = Vec::new();
    if !device.kind.is_empty() {
        details.push(device.kind.clone());
    }
    if !device.address.is_empty() {
        details.push(device.address.clone());
    }
    if device.paired {
        details.push(if device.trusted {
            "Trusted".into()
        } else {
            "Not trusted".into()
        });
    }
    let subtitle = (!details.is_empty()).then(|| details.join(" · ").into());
    let connect_id = device.id.clone();
    let pair_id = device.id.clone();
    let pair_name = SharedString::from(device.name.clone());
    let connect = !device.connected;
    let connect_view = view.clone();
    let pair_view = view.clone();
    let forget_id = device.id.clone();
    let forget_name = SharedString::from(device.name.clone());
    let forget_view = view.clone();
    let action = if device.connected {
        "Disconnect"
    } else if device.paired {
        "Connect"
    } else {
        "Pair"
    };
    row_base()
        .child(tile(
            "icons/bluetooth.svg",
            if device.connected {
                accent()
            } else {
                secondary()
            },
            22.0,
        ))
        .child(text_block(device.name.clone().into(), subtitle))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(
                        SharedString::from(format!("bluetooth-device-action-{connect_id}")),
                        action,
                    )
                    .xsmall()
                    .disabled(busy)
                    .when(device.paired, |button| {
                        button.on_click(move |_, _, cx| {
                            connect_view.update(cx, |settings, cx| {
                                settings.set_bluetooth_device_connected(
                                    connect_id.clone(),
                                    connect,
                                    cx,
                                );
                            });
                        })
                    })
                    .when(!device.paired, |button| {
                        button.on_click(move |_, window, cx| {
                            pair_view.update(cx, |settings, cx| {
                                settings.begin_bluetooth_pairing(
                                    pair_id.clone(),
                                    pair_name.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                    }),
                )
                .when(device.paired, |actions| {
                    actions.child(
                        Button::new(
                            SharedString::from(format!("bluetooth-device-forget-{forget_id}")),
                            "Forget…",
                        )
                        .xsmall()
                        .busy(forgetting)
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            forget_view.update(cx, |settings, cx| {
                                settings.request_bluetooth_forget(
                                    forget_id.clone(),
                                    forget_name.clone(),
                                    cx,
                                );
                            });
                        }),
                    )
                }),
        )
        .into_any_element()
}

pub(super) fn display_layout_preview(outputs: &[rmac_display::Output]) -> Div {
    let enabled = outputs
        .iter()
        .filter_map(|output| Some((output, output.logical.as_ref()?)))
        .collect::<Vec<_>>();
    let Some(min_x) = enabled.iter().map(|(_, logical)| logical.x).min() else {
        return note_card("No enabled display layout is available.");
    };
    let min_y = enabled
        .iter()
        .map(|(_, logical)| logical.y)
        .min()
        .unwrap_or_default();
    let max_x = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.x) + i64::from(logical.width))
        .max()
        .unwrap_or(1);
    let max_y = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.y) + i64::from(logical.height))
        .max()
        .unwrap_or(1);
    let span_x = (max_x - i64::from(min_x)).max(1) as f32;
    let span_y = (max_y - i64::from(min_y)).max(1) as f32;
    let scale = (440.0 / span_x).min(130.0 / span_y);
    let mut canvas = div()
        .relative()
        .w_full()
        .h(px(150.0))
        .rounded(px(9.0))
        .bg(rmac_ui::mac::control_fill())
        .border_1()
        .border_color(sep())
        .overflow_hidden();
    for (index, (output, logical)) in enabled.into_iter().enumerate() {
        let left = 10.0 + (logical.x - min_x) as f32 * scale;
        let top = 10.0 + (logical.y - min_y) as f32 * scale;
        let width = (logical.width as f32 * scale).max(24.0);
        let height = (logical.height as f32 * scale).max(18.0);
        let title = if output.primary {
            format!("{} · Main", index + 1)
        } else {
            format!("{} · {}", index + 1, output.name)
        };
        canvas = canvas.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .flex()
                .items_center()
                .justify_center()
                .px_1()
                .rounded(px(5.0))
                .border_2()
                .border_color(if output.primary { accent() } else { sep() })
                .bg(if output.primary { accent() } else { card_bg() })
                .text_size(rmac_ui::text_px(11.0))
                .text_color(if output.primary {
                    hsl(0xffffff)
                } else {
                    label()
                })
                .overflow_hidden()
                .child(title),
        );
    }
    div()
        .p_2()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(canvas)
}

/// A slider row (state held in its own SliderState entity).
pub(super) fn slider_row(
    title: &'static str,
    state: &Entity<SliderState>,
    value: SharedString,
) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(div().flex_1().child(Slider::new(state).w_full()))
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(value),
        )
}

pub(super) fn balance_slider_row(state: &Entity<SliderState>) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("Balance"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("L"),
                )
                .child(div().flex_1().child(Slider::new(state).w_full()))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("R"),
                ),
        )
        .child(div().w(px(44.0)).flex_none())
}

pub(super) type GtkTextScaleOption = (&'static str, f64);

pub(super) const GTK_TEXT_SCALE_OPTIONS: [GtkTextScaleOption; 3] =
    [("Standard", 1.0), ("Large", 1.2), ("Extra Large", 1.3)];

pub(super) fn dock_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [DockOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().cloned().enumerate() {
        let option_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("{id}-{index}"))),
                option_label,
            )
            .flex_1()
            .selected(selected == Some(index))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                option_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(change.clone(), cx)
                });
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dock_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: Option<SharedString>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> DockChange,
) -> AnyElement {
    let toggle_view = view.clone();
    let toggle = Toggle::new(id)
        .checked(checked)
        .disabled(!enabled)
        .on_click(move |value, _, cx| {
            toggle_view.update(cx, |settings, cx| {
                settings.apply_dock_change(change(*value), cx)
            });
        });
    row_base()
        .child(text_block(title.into(), subtitle))
        .child(toggle)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dock_output_row(
    view: &Entity<Settings>,
    id: &str,
    title: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    enabled: bool,
    scope: rmac_shell_settings::OutputScope,
) -> AnyElement {
    let output_view = view.clone();
    row_base()
        .child(text_block(title, subtitle))
        .child(
            Button::new(
                ElementId::from(SharedString::from(format!("dock-output-{id}"))),
                if selected { "Selected" } else { "Use" },
            )
            .selected(selected)
            .disabled(!enabled || selected)
            .on_click(move |_, _, cx| {
                output_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(DockChange::Outputs(scope.clone()), cx)
                });
            }),
        )
        .into_any_element()
}

pub(super) fn wallpaper_fit_row(
    view: Entity<Settings>,
    selected: rmac_shell_settings::WallpaperFit,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(380.0));
    for (index, (label, fit)) in WALLPAPER_FIT_OPTIONS.iter().copied().enumerate() {
        let fit_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("wallpaper-fit-{index}"))),
                label,
            )
            .flex_1()
            .selected(selected == fit)
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                fit_view.update(cx, |settings, cx| {
                    let target = settings.wallpaper_target.clone();
                    settings.apply_wallpaper_change(target, WallpaperChange::Fit(fit), cx);
                });
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("Display mode"),
        )
        .child(control)
        .into_any_element()
}

pub(super) fn shortcut_configuration_available(
    status: Option<&rmac_shortcuts::BackendStatus>,
) -> bool {
    matches!(
        status,
        Some(rmac_shortcuts::BackendStatus::Portal {
            version,
            can_configure: true,
        }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION
    )
}

pub(super) fn spotlight_provider_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let toggle_view = view.clone();
    row_base()
        .child(text_block(title.into(), Some(subtitle.into())))
        .child(
            Toggle::new(ElementId::from(SharedString::from(format!(
                "spotlight-provider-{id}"
            ))))
            .checked(checked)
            .disabled(!enabled)
            .on_click(move |value, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.apply_spotlight_change(
                        SpotlightChange::ProviderEnabled {
                            id: id.into(),
                            enabled: *value,
                        },
                        cx,
                    )
                });
            }),
        )
        .into_any_element()
}

pub(super) fn theme_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [ThemeOption],
    selected: usize,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{index}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(rmac_ui::text_px(11.0))
                .when(index == selected, |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(index != selected, |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.apply_theme_change(change, cx));
                        })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child(option_label),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

pub(super) fn gtk_text_scale_row(
    view: Entity<Settings>,
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, factor)) in GTK_TEXT_SCALE_OPTIONS.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "gtk-text-scale-{index}"
                ))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(rmac_ui::text_px(11.0))
                .when(selected == Some(index), |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(selected != Some(index), |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.set_gtk_text_scale(factor, cx));
                        })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child(option_label),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("GTK application text"),
        )
        .child(control)
        .into_any_element()
}

pub(super) fn input_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [InputOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{index}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(rmac_ui::text_px(11.0))
                .when(selected == Some(index), |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(selected != Some(index), |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.apply_input_change(change, cx));
                        })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child(option_label),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn input_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> InputChange,
) -> AnyElement {
    let mut switch = Toggle::new(id).checked(checked);
    if enabled {
        switch = switch.on_click(move |value, _, cx| {
            view.update(cx, |settings, cx| {
                settings.apply_input_change(change(*value), cx)
            });
        });
    }
    row_base()
        .child(tile(icon, secondary(), 22.0))
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(
            div()
                .when(!enabled, |element| element.opacity(0.55))
                .child(switch),
        )
        .into_any_element()
}

/// A clickable navigation row that pushes a subpage onto the back stack.
pub(super) fn nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
    target: SubPage,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("nav-{title}")));
    let mut r = row_base()
        .id(id)
        .cursor_pointer()
        .hover(|h| h.bg(rmac_ui::mac::hover()))
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None));
    if let Some(v) = value {
        r = r.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
    }
    r.child(glyph(
        "icons/chevron-right.svg",
        14.0,
        rmac_ui::mac::text_tertiary(),
    ))
    .on_click(move |_, _, cx| {
        let target = target.clone();
        view.update(cx, |s, cx| s.push(target, cx));
    })
    .into_any_element()
}

/// Build a rounded white card from rows, inserting inset separators.
pub(super) fn card(rows: Vec<AnyElement>) -> Div {
    let mut c = div()
        .v_flex()
        .mb_3()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep());
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        c = c.child(r);
        if i + 1 < n {
            c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
        }
    }
    c
}

pub(super) fn battery_history_card(points: &[rmac_power::BatteryHistoryPoint]) -> Div {
    let samples = sample_battery_history(points, 48);
    let minimum = points
        .iter()
        .map(|point| point.percentage)
        .min()
        .unwrap_or_default();
    let maximum = points
        .iter()
        .map(|point| point.percentage)
        .max()
        .unwrap_or_default();
    let latest = points
        .last()
        .map(|point| point.percentage)
        .unwrap_or_default();
    let bars = samples.into_iter().map(|point| {
        let color = if matches!(
            point.state,
            rmac_power::BatteryState::Charging | rmac_power::BatteryState::PendingCharge
        ) {
            hsl(0x34c759)
        } else {
            accent()
        };
        div()
            .flex_1()
            .min_w(px(2.0))
            .h(px(4.0 + f32::from(point.percentage) * 0.72))
            .rounded(px(2.0))
            .bg(color)
    });
    div()
        .v_flex()
        .mb_3()
        .gap_2()
        .p_3()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(format!(
                    "Last 24 hours · {minimum}% minimum · {maximum}% maximum · {latest}% latest"
                )),
        )
        .child(
            div()
                .h(px(80.0))
                .flex()
                .items_end()
                .gap(px(2.0))
                .children(bars),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(rmac_ui::text_px(10.5))
                .text_color(rmac_ui::mac::text_tertiary())
                .child("24 hours ago")
                .child("Now"),
        )
}

/// Format bytes as decimal GB (matching macOS storage display).
pub(super) fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}
