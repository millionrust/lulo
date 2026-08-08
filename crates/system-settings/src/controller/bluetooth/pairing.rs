//! Bluetooth pairing and prompt lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn begin_bluetooth_pairing(
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

    pub(in crate::controller) fn finish_bluetooth_pairing(
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

    pub(in crate::controller) fn submit_bluetooth_pairing_prompt(
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

    pub(in crate::controller) fn reject_bluetooth_pairing_prompt(
        &mut self,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::controller) fn cancel_bluetooth_pairing(&mut self, cx: &mut Context<Self>) {
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
}
