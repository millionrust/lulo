//! Quick Settings reusable binary tile, error, icon, and Focus projection.

use super::*;

impl QuickSettingsView {
    pub(super) fn icon_badge(path: &'static str, active: bool) -> AnyElement {
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

    pub(super) fn control_error(
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

    pub(super) fn binary_tile(
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

    pub(super) fn focus_tile(&self, focus: &Tile<FocusValue>, cx: &Context<Self>) -> AnyElement {
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
