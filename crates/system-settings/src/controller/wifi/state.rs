//! Wi-Fi snapshot, radio, stream, and refresh lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn apply_wifi_snapshot(
        &mut self,
        snapshot: rmac_network::WifiSnapshot,
    ) {
        self.wifi_available = snapshot.available;
        self.wifi_on = snapshot.enabled;
        self.wifi_interface = snapshot.interface;
        self.wifi_networks = snapshot.networks;
        self.wifi_saved_networks = snapshot.saved_networks;
    }

    pub(in crate::controller) fn begin_wifi_mutation(&mut self) {
        self.wifi_generation = self.wifi_generation.wrapping_add(1);
        self.wifi_busy = true;
    }

    pub(in crate::controller) fn finish_wifi_stream_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_stream_error = None;
            }
            Err(_) => {
                self.wifi_stream_error =
                    Some("Live Wi-Fi state could not be refreshed from NetworkManager".into());
            }
        }
    }

    pub(in crate::controller) fn finish_wifi_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_connecting = None;
        self.wifi_cancellation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_error = None;
            }
            Err(error) => {
                self.wifi_error = Some(format!("Could not update Wi-Fi: {error}").into());
            }
        }
    }

    pub(in crate::controller) fn finish_wifi_password_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
        recovery_snapshot: Option<rmac_network::WifiSnapshot>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_connecting = None;
        self.wifi_cancellation = None;
        if result.is_err() {
            if let Some(snapshot) = recovery_snapshot {
                self.apply_wifi_snapshot(snapshot);
            }
        }
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_password_prompt = None;
                self.wifi_error = None;
            }
            Err(error) if error.is_cancelled() => {
                self.wifi_password_prompt = None;
                self.wifi_error = None;
            }
            Err(error) => {
                if let Some(prompt) = &mut self.wifi_password_prompt {
                    prompt.validation_error =
                        Some(format!("Could not join this network: {error}").into());
                } else {
                    self.wifi_error = Some(format!("Could not join Wi-Fi: {error}").into());
                }
            }
        }
    }

    pub(in crate::controller) fn finish_wifi_enterprise_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
        recovery_snapshot: Option<rmac_network::WifiSnapshot>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_connecting = None;
        self.wifi_cancellation = None;
        if result.is_err() {
            if let Some(snapshot) = recovery_snapshot {
                self.apply_wifi_snapshot(snapshot);
            }
        }
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_enterprise_prompt = None;
                self.wifi_error = None;
            }
            Err(error) if error.is_cancelled() => {
                self.wifi_enterprise_prompt = None;
                self.wifi_error = None;
            }
            Err(_) => {
                if let Some(prompt) = &mut self.wifi_enterprise_prompt {
                    prompt.validation_error = Some(
                        "Could not join securely. Verify the identity, password, certificate domain, and that the network uses PEAP with MSCHAPv2."
                            .into(),
                    );
                } else {
                    self.wifi_error = Some("Could not join enterprise Wi-Fi.".into());
                }
            }
        }
    }

    pub(in crate::controller) fn finish_wifi_forget_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
        recovery_snapshot: Option<rmac_network::WifiSnapshot>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_forgetting = None;
        self.wifi_forget_confirmation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_error = None;
            }
            Err(error) => {
                if let Some(snapshot) = recovery_snapshot {
                    self.apply_wifi_snapshot(snapshot);
                }
                self.wifi_error = Some(format!("Could not forget Wi-Fi network: {error}").into());
            }
        }
    }

    pub(in crate::controller) fn set_wifi_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy || self.wifi_loading || !self.wifi_available {
            return;
        }
        self.begin_wifi_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_network::set_enabled(enabled)?;
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn refresh_wifi(&mut self, cx: &mut Context<Self>) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        self.begin_wifi_mutation();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    rmac_network::request_scan()?;
                    std::thread::sleep(Duration::from_millis(750));
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }
}
