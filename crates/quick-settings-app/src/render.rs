mod cards;
mod controls;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, svg, AnyElement, Context, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use gpui_component::StyledExt as _;
use rmac_quick_settings::accessibility::{
    CHANGING_LABEL, DISMISS_LABEL, READING_SYSTEM_STATE_LABEL, SYSTEM_SETTINGS_LABEL,
};
use rmac_quick_settings::{Command, Control, FocusValue, PowerValue, SoundValue, Tile};
use rmac_ui::{mac, Button, ButtonRole, Progress, Slider, Toggle};

use crate::view::QuickSettingsView;

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
            .rounded(px(mac::radius_popover()))
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .bg(mac::material())
            .text_color(mac::text())
            .when_some(self.stream_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .mx_3()
                        .mt_2()
                        .px_3()
                        .py_2()
                        .rounded(px(mac::radius_control()))
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
                        .rounded(px(mac::radius_control()))
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
                    .child(
                        div()
                            .h(px(166.0))
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .v_flex()
                                    .gap_2()
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
                                    .child(self.focus_tile(&view.focus, cx)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(self.power_card(&view.power, cx)),
                            ),
                    )
                    .child(self.sound_card(&view.sound, cx)),
            )
            .child(
                div()
                    .h(px(40.0))
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
