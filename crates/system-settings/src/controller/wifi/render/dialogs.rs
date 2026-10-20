//! Wi-Fi credential, enterprise, and forget dialogs.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_wifi_forget_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.wifi_forget_confirmation.as_ref()?;
        let message = format!(
            "This computer will remove every accessible saved profile for “{}”. If the network is active, it will disconnect. You will need its password to join again.",
            prompt.ssid
        );
        Some(
            rmac_ui::alert(
                "Forget This Network?",
                message,
                vec![
                    rmac_ui::dialog_button(
                        "wifi-forget-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_forget(cx)))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "wifi-forget-confirm",
                        "Forget",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_wifi_forget(cx)))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }
    pub(in crate::controller) fn render_wifi_password_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.wifi_password_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };

        let content = div()
            .w(px(380.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enter the password for this Wi-Fi network."),
                    ),
            )
            .child(TextField::new(&prompt.editor).disabled(busy).w_full())
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Connecting securely…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-password-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_password(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-password-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_password(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-password-dialog", content)
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_password(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_password(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(in crate::controller) fn render_wifi_enterprise_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.wifi_enterprise_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };
        let field = |label_text: &'static str, editor: &Entity<InputState>| {
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child(label_text),
                )
                .child(TextField::new(editor).disabled(busy).w_full())
        };

        let content = div()
            .w(px(430.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enterprise Wi-Fi · PEAP with MSCHAPv2"),
                    ),
            )
            .child(field("Identity", &prompt.identity))
            .child(field(
                "Anonymous outer identity (optional)",
                &prompt.anonymous_identity,
            ))
            .child(field(
                "Server certificate domain",
                &prompt.certificate_domain,
            ))
            .child(field("Password", &prompt.password))
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child(
                        "The authentication server must chain to the system trust store and its certificate must match this domain. An anonymous outer identity can avoid exposing the login identity before the secure tunnel forms. Ask your network administrator for both values.",
                    ),
            )
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Verifying and connecting…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-enterprise-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_enterprise(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-enterprise-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_enterprise(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-enterprise-dialog", content)
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_enterprise(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_enterprise(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    /// Wi-Fi ▸ Other…: Network Name, Security (None or WPA2/WPA3 Personal),
    /// Password, Cancel and Join, as on macOS 26.
    pub(in crate::controller) fn render_wifi_other_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.wifi_other_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let view = cx.entity();
        let field = |label_text: &'static str, child: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(96.0))
                        .flex_none()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(label())
                        .child(label_text),
                )
                .child(div().flex_1().child(child))
        };
        let secure_view = view.clone();
        let open_view = view.clone();
        let security = form_popup(
            "wifi-other-security",
            if prompt.secure {
                "WPA2/WPA3 Personal".into()
            } else {
                "None".into()
            },
            vec![
                choice("None", !prompt.secure, move |_, cx| {
                    open_view.update(cx, |settings, cx| settings.set_wifi_other_secure(false, cx));
                }),
                choice("WPA2/WPA3 Personal", prompt.secure, move |_, cx| {
                    secure_view.update(cx, |settings, cx| settings.set_wifi_other_secure(true, cx));
                }),
            ],
            !busy,
        )
        .accessible_name("Security");
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_3()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child("Find and join a Wi-Fi network."),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(
                                "Enter the name and security type of the network you want to join.",
                            ),
                    ),
            )
            .child(field(
                "Network Name",
                TextField::new(&prompt.name)
                    .disabled(busy)
                    .w_full()
                    .into_any_element(),
            ))
            .child(field("Security", security.into_any_element()))
            .when(prompt.secure, |dialog| {
                dialog.child(field(
                    "Password",
                    TextField::new(&prompt.password)
                        .disabled(busy)
                        .w_full()
                        .into_any_element(),
                ))
            })
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Joining…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-other-cancel",
                            if busy { "Stop" } else { "Cancel" },
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_other(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-other-join",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.submit_wifi_other(window, cx)
                                }),
                            ),
                        )
                    }),
            );
        Some(
            rmac_ui::dialog("wifi-other-dialog", content)
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_other(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_other(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}
