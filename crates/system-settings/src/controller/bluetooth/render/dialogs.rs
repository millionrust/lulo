//! Bluetooth pairing and forget dialogs.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_bluetooth_forget_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.bluetooth_forget_confirmation.as_ref()?;
        Some(
            rmac_ui::alert(
                "Forget This Device?",
                format!(
                    "This computer will remove pairing information for “{}” and disconnect it. You will need to pair it again to reconnect.",
                    prompt.name
                ),
                vec![
                    rmac_ui::dialog_button(
                        "bluetooth-forget-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_bluetooth_forget(cx)
                    }))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "bluetooth-forget-confirm",
                        "Forget",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_bluetooth_forget(cx)
                    }))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }
    pub(in crate::controller) fn render_bluetooth_pairing_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let pairing = self.bluetooth_pairing.as_ref()?;
        let prompt = pairing.prompt.as_ref();
        let requires_input = prompt.is_some_and(|prompt| {
            matches!(
                prompt.kind,
                rmac_bluetooth::PairingPromptKind::EnterPinCode
                    | rmac_bluetooth::PairingPromptKind::EnterPasskey
            )
        });
        let (instruction, code, primary_label) = if let Some(prompt) = prompt {
            match &prompt.kind {
                rmac_bluetooth::PairingPromptKind::ConfirmPasskey { passkey } => (
                    "Make sure this code is also shown on the Bluetooth device.",
                    Some(format!("{passkey:06}")),
                    "Pair",
                ),
                rmac_bluetooth::PairingPromptKind::EnterPinCode => (
                    "Enter the 1–16 character PIN supplied by the device.",
                    None,
                    "Continue",
                ),
                rmac_bluetooth::PairingPromptKind::EnterPasskey => (
                    "Enter the six-digit passkey shown on the device.",
                    None,
                    "Continue",
                ),
                rmac_bluetooth::PairingPromptKind::AuthorizePairing => (
                    "Allow this device to pair with this computer?",
                    None,
                    "Pair",
                ),
                rmac_bluetooth::PairingPromptKind::AuthorizeService { .. } => (
                    "Allow this paired device to use its requested Bluetooth service?",
                    None,
                    "Allow",
                ),
            }
        } else if let Some(display) = &pairing.display {
            match display {
                BluetoothPairingDisplay::PinCode(pin_code) => (
                    "Type this code on the Bluetooth device, then finish there.",
                    Some(pin_code.clone()),
                    "",
                ),
                BluetoothPairingDisplay::Passkey { passkey, entered } => (
                    "Type this code on the Bluetooth device, then finish there.",
                    Some(format!("{passkey:06} · {entered}/6 entered")),
                    "",
                ),
            }
        } else {
            ("Keep the device nearby and ready to pair.", None, "")
        };

        let content = div()
            .w(px(400.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
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
                            .child(format!("Connect to “{}”?", pairing.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(instruction),
                    ),
            )
            .when_some(code, |dialog, code| {
                dialog.child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(rmac_ui::text_px(25.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(label())
                        .child(code),
                )
            })
            .when(requires_input, |dialog| {
                dialog.child(TextField::new(&pairing.editor).w_full())
            })
            .when_some(pairing.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(prompt.is_none(), |dialog| {
                dialog.child(Progress::indeterminate().label(if pairing.stopping {
                    "Ending pairing…"
                } else {
                    "Pairing securely…"
                }))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "bluetooth-pairing-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(pairing.stopping)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_bluetooth_pairing(cx))),
                    )
                    .when(prompt.is_some(), |buttons| {
                        buttons
                            .child(
                                rmac_ui::dialog_button(
                                    "bluetooth-pairing-reject",
                                    "Don’t Pair",
                                    rmac_ui::DialogButtonKind::Normal,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.reject_bluetooth_pairing_prompt(cx),
                                )),
                            )
                            .child(
                                rmac_ui::dialog_button(
                                    "bluetooth-pairing-submit",
                                    primary_label,
                                    rmac_ui::DialogButtonKind::Primary,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.submit_bluetooth_pairing_prompt(window, cx)
                                    },
                                )),
                            )
                    }),
            );

        Some(
            rmac_ui::dialog("bluetooth-pairing-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_bluetooth_pairing(cx);
                        }
                        "enter"
                            if this
                                .bluetooth_pairing
                                .as_ref()
                                .is_some_and(|pairing| pairing.prompt.is_some()) =>
                        {
                            cx.stop_propagation();
                            this.submit_bluetooth_pairing_prompt(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}
