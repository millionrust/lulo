//! Bluetooth snapshot, adapter, discovery, and connection lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_bluetooth_update(
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

    pub(in crate::controller) fn finish_bluetooth_stream_update(
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

    pub(in crate::controller) fn apply_bluetooth_snapshot(
        &mut self,
        snapshot: rmac_bluetooth::Snapshot,
    ) {
        self.bluetooth_available = snapshot.available;
        self.bluetooth_on = snapshot.powered;
        self.bt_discoverable = snapshot.discoverable;
        self.bluetooth_discovering = snapshot.discovering;
        self.bluetooth_adapter_name = snapshot.adapter_name;
        self.bt_devices = snapshot.devices;
    }

    pub(in crate::controller) fn begin_bluetooth_mutation(&mut self) {
        self.bluetooth_generation = self.bluetooth_generation.wrapping_add(1);
        self.bluetooth_busy = true;
    }

    pub(in crate::controller) fn set_bluetooth_powered(
        &mut self,
        powered: bool,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::controller) fn set_bluetooth_discoverable(
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

    pub(in crate::controller) fn refresh_bluetooth(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn set_bluetooth_device_connected(
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
}
