use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};
use gpui_component::StyledExt as _;
use rmac_quick_settings::accessibility::{
    CHANGING_LABEL, DISMISS_LABEL, QUICK_SETTINGS_DESCRIPTION, QUICK_SETTINGS_TITLE,
    READING_SYSTEM_STATE_LABEL, SYSTEM_SETTINGS_LABEL,
};
use rmac_quick_settings::{Command, Control, FocusValue, PowerValue, SoundValue, Tile};
use rmac_ui::{Button, ButtonRole, Progress, Slider, Toggle, mac};

use crate::view::QuickSettingsView;

impl QuickSettingsView {
    fn icon_badge(path: &'static str, active: bool) -> AnyElement {
        div()
            .size(px(34.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(if active {
                mac::accent()
            } else {
                mac::control_fill()
            })
            .text_color(if active {
                mac::on_accent()
            } else {
                mac::text_secondary()
            })
            .child(svg().path(path).size(px(17.0)).text_color(if active {
                mac::on_accent()
            } else {
                mac::text_secondary()
            }))
            .into_any_element()
    }

    fn control_error(
        &self,
        control: Control,
        error: Option<String>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        error.map(|error| {
            let view = cx.entity();
            div()
                .mt_2()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(7.0))
                .bg(mac::error_background())
                .text_size(rmac_ui::text_px(10.0))
                .text_color(mac::danger())
                .child(div().flex_1().min_w_0().child(error))
                .child(
                    Button::new(
                        SharedString::from(format!("dismiss-{control:?}-error")),
                        DISMISS_LABEL,
                    )
                    .ghost()
                    .xsmall()
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| this.dismiss_error(control, cx));
                    }),
                )
                .into_any_element()
        })
    }

    fn binary_tile(
        &self,
        control: Control,
        title: &'static str,
        symbol: &'static str,
        tile: &Tile<bool>,
        command: fn(bool) -> Command,
        cx: &Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let disabled = !tile.available || tile.busy;
        div()
            .w_full()
            .rounded(px(12.0))
            .border_1()
            .border_color(if tile.value && tile.available {
                mac::accent_border()
            } else {
                mac::separator()
            })
            .bg(if tile.value && tile.available {
                mac::accent_subtle()
            } else {
                mac::raised()
            })
            .px_3()
            .py_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::icon_badge(symbol, tile.value && tile.available))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(if tile.busy {
                                        CHANGING_LABEL.to_owned()
                                    } else {
                                        tile.summary.clone()
                                    }),
                            ),
                    )
                    .child(
                        Toggle::new(SharedString::from(format!("quick-{control:?}")))
                            .checked(tile.value)
                            .disabled(disabled)
                            .tooltip(title)
                            .on_click(move |value, _, cx| {
                                view.update(cx, |this, cx| this.execute(command(*value), cx));
                            }),
                    ),
            )
            .when_some(
                self.control_error(control, tile.error.clone(), cx),
                |tile, error| tile.child(error),
            )
            .into_any_element()
    }

    fn sound_card(&self, sound: &Tile<SoundValue>, cx: &Context<Self>) -> AnyElement {
        let mute_view = cx.entity();
        let disabled = !sound.available || sound.busy;
        div()
            .w_full()
            .rounded(px(12.0))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::raised())
            .px_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::icon_badge(
                        "icons/volume-2.svg",
                        sound.available && !sound.value.muted,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child("Sound"),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(if sound.busy {
                                        CHANGING_LABEL.to_owned()
                                    } else {
                                        sound.summary.clone()
                                    }),
                            ),
                    )
                    .child(
                        Toggle::new("quick-sound-mute")
                            .checked(sound.value.muted)
                            .disabled(disabled)
                            .tooltip("Mute output")
                            .on_click(move |value, _, cx| {
                                mute_view.update(cx, |this, cx| {
                                    this.execute(Command::SetOutputMuted(*value), cx)
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(72.0))
                            .text_size(rmac_ui::text_px(10.5))
                            .text_color(mac::text_secondary())
                            .child("Output volume"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Slider::new(&self.volume).disabled(disabled).w_full()),
                    )
                    .child(
                        div()
                            .w(px(34.0))
                            .text_right()
                            .text_size(rmac_ui::text_px(10.5))
                            .text_color(mac::text_secondary())
                            .child(format!("{}%", sound.value.volume)),
                    ),
            )
            .when_some(
                self.control_error(Control::Sound, sound.error.clone(), cx),
                |card, error| card.child(error),
            )
            .into_any_element()
    }

    fn power_card(&self, power: &Tile<PowerValue>, cx: &Context<Self>) -> AnyElement {
        let buttons = power
            .value
            .supported
            .iter()
            .copied()
            .map(|profile| {
                let view = cx.entity();
                Button::new(
                    SharedString::from(format!("quick-power-{}", profile.id())),
                    profile.label(),
                )
                .xsmall()
                .role(if power.value.active == Some(profile) {
                    ButtonRole::Primary
                } else {
                    ButtonRole::Secondary
                })
                .disabled(!power.available || power.busy)
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.execute(Command::SetPowerProfile(profile), cx)
                    });
                })
            })
            .collect::<Vec<_>>();
        div()
            .w_full()
            .rounded(px(12.0))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::raised())
            .px_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::icon_badge(
                        "icons/battery-charging.svg",
                        power.available,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child("Power Mode"),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(if power.busy {
                                        CHANGING_LABEL.to_owned()
                                    } else {
                                        power.summary.clone()
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .when(buttons.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(rmac_ui::text_px(10.5))
                                .text_color(mac::text_secondary())
                                .child("No power profiles are available"),
                        )
                    })
                    .children(buttons),
            )
            .when_some(
                self.control_error(Control::Power, power.error.clone(), cx),
                |card, error| card.child(error),
            )
            .into_any_element()
    }

    fn focus_tile(&self, focus: &Tile<FocusValue>, cx: &Context<Self>) -> AnyElement {
        let tile = Tile {
            available: focus.available,
            busy: focus.busy,
            value: focus.value.enabled,
            summary: focus.summary.clone(),
            error: focus.error.clone(),
        };
        self.binary_tile(
            Control::Focus,
            "Focus",
            "icons/moon.svg",
            &tile,
            Command::SetFocusEnabled,
            cx,
        )
    }
}

impl Render for QuickSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.state.view();
        let settings_view = cx.entity();
        let dismiss_operation_error = cx.entity();
        div()
            .size_full()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.dismiss(window, cx);
                }
            }))
            .v_flex()
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .bg(mac::window())
            .text_color(mac::text())
            .child(
                div()
                    .h(px(58.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(17.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child(QUICK_SETTINGS_TITLE),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(QUICK_SETTINGS_DESCRIPTION),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(mac::text_tertiary())
                            .child("Esc Close"),
                    ),
            )
            .when_some(self.stream_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .mx_3()
                        .mt_2()
                        .px_3()
                        .py_2()
                        .rounded(px(8.0))
                        .bg(mac::warning_background())
                        .border_1()
                        .border_color(mac::warning_border())
                        .text_size(rmac_ui::text_px(10.5))
                        .text_color(mac::warning_text())
                        .child(error),
                )
            })
            .when_some(self.operation_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .mx_3()
                        .mt_2()
                        .px_3()
                        .py_2()
                        .rounded(px(8.0))
                        .bg(mac::error_background())
                        .border_1()
                        .border_color(mac::error_border())
                        .text_size(rmac_ui::text_px(10.5))
                        .text_color(mac::danger())
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(error))
                        .child(
                            Button::new("dismiss-quick-settings-error", DISMISS_LABEL)
                                .ghost()
                                .xsmall()
                                .on_click(move |_, _, cx| {
                                    dismiss_operation_error
                                        .update(cx, |this, cx| this.dismiss_operation_error(cx));
                                }),
                        ),
                )
            })
            .child(
                div()
                    .id("quick-settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .py_3()
                    .v_flex()
                    .gap_2()
                    .when(!self.received_snapshot, |content| {
                        content.child(
                            div()
                                .h(px(54.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Progress::indeterminate().label(READING_SYSTEM_STATE_LABEL)),
                        )
                    })
                    .child(self.binary_tile(
                        Control::Wifi,
                        "Wi-Fi",
                        "icons/wifi.svg",
                        &view.wifi,
                        Command::SetWifiEnabled,
                        cx,
                    ))
                    .child(self.binary_tile(
                        Control::Bluetooth,
                        "Bluetooth",
                        "icons/bluetooth.svg",
                        &view.bluetooth,
                        Command::SetBluetoothPowered,
                        cx,
                    ))
                    .child(self.sound_card(&view.sound, cx))
                    .child(self.power_card(&view.power, cx))
                    .child(self.focus_tile(&view.focus, cx)),
            )
            .child(
                div()
                    .h(px(46.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px_3()
                    .border_t_1()
                    .border_color(mac::separator())
                    .child(
                        Button::new("open-system-settings", SYSTEM_SETTINGS_LABEL)
                            .ghost()
                            .xsmall()
                            .on_click(move |_, _, cx| {
                                settings_view.update(cx, |this, cx| this.open_settings(cx));
                            }),
                    ),
            )
    }
}
