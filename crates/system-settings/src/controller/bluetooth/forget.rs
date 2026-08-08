//! Bluetooth reviewed-forget lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn request_bluetooth_forget(
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

    pub(in crate::controller) fn cancel_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
        if !self.bluetooth_busy {
            self.bluetooth_forget_confirmation = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn confirm_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
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
}
