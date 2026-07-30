//! Bluetooth adapter, device, pairing, and forgetting lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_bluetooth_update(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
    ) {
        self.bluetooth_loading = false;
        self.bluetooth_busy = false;
        match result {
            Ok(snapshot) => {
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_error = None;
            }
            Err(error) => {
                self.bluetooth_error = Some(format!("Could not update Bluetooth: {error}").into());
            }
        }
    }

    pub(super) fn finish_bluetooth_stream_update(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_stream_error = None;
            }
            Err(error) => {
                self.bluetooth_stream_error =
                    Some(format!("Could not refresh live Bluetooth state: {error}").into());
            }
        }
    }

    pub(super) fn apply_bluetooth_snapshot(&mut self, snapshot: rmac_bluetooth::Snapshot) {
        self.bluetooth_available = snapshot.available;
        self.bluetooth_on = snapshot.powered;
        self.bt_discoverable = snapshot.discoverable;
        self.bluetooth_discovering = snapshot.discovering;
        self.bluetooth_adapter_name = snapshot.adapter_name;
        self.bt_devices = snapshot.devices;
    }

    pub(super) fn begin_bluetooth_mutation(&mut self) {
        self.bluetooth_generation = self.bluetooth_generation.wrapping_add(1);
        self.bluetooth_busy = true;
    }

    pub(super) fn set_bluetooth_powered(&mut self, powered: bool, cx: &mut Context<Self>) {
        if self.bluetooth_busy || self.bluetooth_loading || !self.bluetooth_available {
            return;
        }
        self.begin_bluetooth_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_powered(powered)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_bluetooth_discoverable(
        &mut self,
        discoverable: bool,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy || !self.bluetooth_available || !self.bluetooth_on {
            return;
        }
        self.begin_bluetooth_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_discoverable(discoverable)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_bluetooth(&mut self, cx: &mut Context<Self>) {
        if self.bluetooth_busy || !self.bluetooth_available || !self.bluetooth_on {
            return;
        }
        self.begin_bluetooth_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let initial = rmac_bluetooth::snapshot()?;
                    let started = !initial.discovering;
                    if started {
                        rmac_bluetooth::start_discovery()?;
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                    if started {
                        rmac_bluetooth::stop_discovery()?;
                    }
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_bluetooth_device_connected(
        &mut self,
        device_id: String,
        connected: bool,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy {
            return;
        }
        self.begin_bluetooth_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_connected(&device_id, connected)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn begin_bluetooth_pairing(
        &mut self,
        device_id: String,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy
            || self.bluetooth_loading
            || !self.bluetooth_available
            || !self.bluetooth_on
            || !self
                .bt_devices
                .iter()
                .any(|device| device.id == device_id && !device.paired)
        {
            return;
        }

        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder("PIN or passkey")
        });
        let editor_focus = editor.read(cx).focus_handle(cx);
        let (session, events) = rmac_bluetooth::PairingSession::new();
        self.begin_bluetooth_mutation();
        let pairing_generation = self.bluetooth_generation;
        self.bluetooth_pairing = Some(BluetoothPairingState {
            device_id: device_id.clone(),
            name,
            session: session.clone(),
            prompt: None,
            display: None,
            editor,
            validation_error: None,
            stopping: false,
        });
        self.bluetooth_error = None;
        window.focus(&editor_focus);
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if !this.bluetooth_busy || this.bluetooth_generation != pairing_generation {
                            return;
                        }
                        let Some(pairing) = &mut this.bluetooth_pairing else {
                            return;
                        };
                        match event {
                            rmac_bluetooth::PairingEvent::Prompt(prompt) => {
                                pairing.prompt = Some(prompt);
                                pairing.display = None;
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::DisplayPinCode { pin_code } => {
                                pairing.prompt = None;
                                pairing.display = Some(BluetoothPairingDisplay::PinCode(pin_code));
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::DisplayPasskey { passkey, entered } => {
                                pairing.prompt = None;
                                pairing.display =
                                    Some(BluetoothPairingDisplay::Passkey { passkey, entered });
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::TimedOut => {
                                pairing.prompt = None;
                                pairing.display = None;
                                pairing.validation_error =
                                    Some("Pairing confirmation timed out.".into());
                                pairing.stopping = true;
                            }
                            rmac_bluetooth::PairingEvent::Canceled => {
                                pairing.prompt = None;
                                pairing.display = None;
                                pairing.validation_error = None;
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_bluetooth::pair(&device_id, &session);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_bluetooth::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_pairing(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_bluetooth_pairing(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
        recovery_snapshot: Option<rmac_bluetooth::Snapshot>,
    ) {
        self.bluetooth_loading = false;
        self.bluetooth_busy = false;
        self.bluetooth_pairing = None;
        match result {
            Ok(snapshot) => {
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_error = None;
            }
            Err(error) => {
                if let Some(snapshot) = recovery_snapshot {
                    self.apply_bluetooth_snapshot(snapshot);
                }
                if error.is_canceled() || error.is_rejected() {
                    self.bluetooth_error = None;
                } else {
                    self.bluetooth_error =
                        Some(format!("Could not pair Bluetooth device: {error}").into());
                }
            }
        }
    }

    pub(super) fn submit_bluetooth_pairing_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pairing) = &self.bluetooth_pairing else {
            return;
        };
        let Some(prompt) = pairing.prompt.clone() else {
            return;
        };
        let submitted = match prompt.kind {
            rmac_bluetooth::PairingPromptKind::EnterPinCode => {
                let value = pairing.editor.read(cx).value().to_string();
                match rmac_bluetooth::PairingPinCode::new(value) {
                    Ok(value) => pairing.session.submit_pin_code(prompt.id, value),
                    Err(error) => {
                        if let Some(pairing) = &mut self.bluetooth_pairing {
                            pairing.validation_error = Some(error.to_string().into());
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            rmac_bluetooth::PairingPromptKind::EnterPasskey => {
                let value = pairing.editor.read(cx).value().to_string();
                match rmac_bluetooth::PairingPasskey::new(value) {
                    Ok(value) => pairing.session.submit_passkey(prompt.id, value),
                    Err(error) => {
                        if let Some(pairing) = &mut self.bluetooth_pairing {
                            pairing.validation_error = Some(error.to_string().into());
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            _ => pairing.session.accept(prompt.id),
        };

        if let Some(pairing) = &mut self.bluetooth_pairing {
            if submitted {
                let empty_editor = cx.new(|cx| {
                    InputState::new(window, cx)
                        .clean_on_escape()
                        .placeholder("PIN or passkey")
                });
                let focus = empty_editor.read(cx).focus_handle(cx);
                pairing.editor = empty_editor;
                pairing.prompt = None;
                pairing.display = None;
                pairing.validation_error = None;
                window.focus(&focus);
            } else {
                pairing.validation_error = Some("This pairing request has expired.".into());
            }
        }
        cx.notify();
    }

    pub(super) fn reject_bluetooth_pairing_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(pairing) = &mut self.bluetooth_pairing else {
            return;
        };
        let Some(prompt) = pairing.prompt.take() else {
            return;
        };
        if pairing.session.reject(prompt.id) {
            pairing.display = None;
            pairing.validation_error = None;
            pairing.stopping = true;
        } else {
            pairing.validation_error = Some("This pairing request has expired.".into());
        }
        cx.notify();
    }

    pub(super) fn cancel_bluetooth_pairing(&mut self, cx: &mut Context<Self>) {
        let Some(pairing) = &self.bluetooth_pairing else {
            return;
        };
        if pairing.stopping {
            return;
        }
        pairing.session.cancel();
        let device_id = pairing.device_id.clone();
        if let Some(pairing) = &mut self.bluetooth_pairing {
            pairing.prompt = None;
            pairing.display = None;
            pairing.validation_error = None;
            pairing.stopping = true;
        }
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_bluetooth::cancel_pairing(&device_id);
            })
            .detach();
        cx.notify();
    }

    pub(super) fn request_bluetooth_forget(
        &mut self,
        device_id: String,
        name: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy
            || self.bluetooth_loading
            || !self
                .bt_devices
                .iter()
                .any(|device| device.id == device_id && device.paired)
        {
            return;
        }
        self.bluetooth_forget_confirmation = Some(BluetoothForgetPrompt { device_id, name });
        self.bluetooth_error = None;
        cx.notify();
    }

    pub(super) fn cancel_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
        if !self.bluetooth_busy {
            self.bluetooth_forget_confirmation = None;
            cx.notify();
        }
    }

    pub(super) fn confirm_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
        if self.bluetooth_busy || self.bluetooth_loading {
            return;
        }
        let Some(device_id) = self
            .bluetooth_forget_confirmation
            .as_ref()
            .map(|prompt| prompt.device_id.clone())
        else {
            return;
        };
        if !self
            .bt_devices
            .iter()
            .any(|device| device.id == device_id && device.paired)
        {
            self.bluetooth_forget_confirmation = None;
            self.bluetooth_error = Some("The Bluetooth device is no longer paired.".into());
            cx.notify();
            return;
        }

        self.begin_bluetooth_mutation();
        self.bluetooth_forgetting = Some(device_id.clone());
        self.bluetooth_forget_confirmation = None;
        self.bluetooth_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_bluetooth::remove_device(&device_id);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_bluetooth::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.bluetooth_loading = false;
                this.bluetooth_busy = false;
                this.bluetooth_forgetting = None;
                match result {
                    Ok(snapshot) => {
                        this.apply_bluetooth_snapshot(snapshot);
                        this.bluetooth_error = None;
                    }
                    Err(error) => {
                        if let Some(snapshot) = recovery_snapshot {
                            this.apply_bluetooth_snapshot(snapshot);
                        }
                        this.bluetooth_error =
                            Some(format!("Could not forget Bluetooth device: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_bluetooth_forget_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
    pub(super) fn render_bluetooth(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let power_subtitle = if self.bluetooth_loading {
            Some("Reading system state…".into())
        } else if self.bluetooth_busy {
            Some("Applying change…".into())
        } else {
            self.bluetooth_adapter_name.clone().map(Into::into)
        };
        let power_view = view.clone();
        let power = Toggle::new("bluetooth-power")
            .checked(self.bluetooth_on)
            .on_click(move |powered, _, cx| {
                power_view.update(cx, |settings, cx| {
                    settings.set_bluetooth_powered(*powered, cx)
                });
            });
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/bluetooth.svg", accent(), 22.0))
            .child(text_block("Bluetooth".into(), power_subtitle))
            .child(power)
            .into_any_element()])];

        if self.bluetooth_loading {
            cards.push(note_card("Loading Bluetooth state from the system…"));
            return self.pane(cards);
        }
        if !self.bluetooth_available {
            cards.push(note_card(
                "No Bluetooth adapter is available through the system Bluetooth service.",
            ));
            return self.pane(cards);
        }

        if self.bluetooth_on {
            let discoverable_view = view.clone();
            let discoverable = Toggle::new("bluetooth-discoverable")
                .checked(self.bt_discoverable)
                .on_click(move |enabled, _, cx| {
                    discoverable_view.update(cx, |settings, cx| {
                        settings.set_bluetooth_discoverable(*enabled, cx)
                    });
                });
            cards.push(card(vec![row_base()
                .child(tile("icons/bluetooth.svg", secondary(), 22.0))
                .child(text_block(
                    "Discoverable".into(),
                    Some("Allow nearby devices to find this computer.".into()),
                ))
                .child(discoverable)
                .into_any_element()]));

            let refresh_view = view.clone();
            let refresh_label = if self.bluetooth_busy || self.bluetooth_discovering {
                "Scanning…"
            } else {
                "Refresh"
            };
            cards.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(secondary())
                            .child("Devices"),
                    )
                    .child(
                        div()
                            .id("bluetooth-refresh")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .child(refresh_label)
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_bluetooth(cx));
                            }),
                    ),
            );

            let connected: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.connected)
                .map(|device| {
                    bluetooth_device_row(
                        &view,
                        device,
                        self.bluetooth_busy,
                        self.bluetooth_forgetting.as_deref() == Some(device.id.as_str()),
                    )
                })
                .collect();
            if !connected.is_empty() {
                cards.push(section_header("Connected"));
                cards.push(card(connected));
            }

            let known: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.paired && !device.connected)
                .map(|device| {
                    bluetooth_device_row(
                        &view,
                        device,
                        self.bluetooth_busy,
                        self.bluetooth_forgetting.as_deref() == Some(device.id.as_str()),
                    )
                })
                .collect();
            if !known.is_empty() {
                cards.push(section_header("Known Devices"));
                cards.push(card(known));
            }

            let nearby: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| !device.paired && !device.connected)
                .map(|device| bluetooth_device_row(&view, device, self.bluetooth_busy, false))
                .collect();
            if !nearby.is_empty() {
                cards.push(section_header("Nearby Devices"));
                cards.push(card(nearby));
            }
            if self.bt_devices.is_empty() {
                cards.push(note_card(
                    "No Bluetooth devices found. Refresh to scan again.",
                ));
            }
            cards.push(note_card(
                "Pairing uses a one-transaction confirmation agent. Confirm that displayed codes match; paired devices become trusted only after BlueZ reports success.",
            ));
        }
        self.pane(cards)
    }

    pub(super) fn render_bluetooth_pairing_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
