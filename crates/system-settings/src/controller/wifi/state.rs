//! Wi-Fi snapshot, radio, stream, and refresh lifecycle.

use super::*;

/// See [`Settings::wifi_view_state`].
pub(in crate::controller) type WifiViewState = (
    bool,
    bool,
    bool,
    bool,
    Option<String>,
    Vec<rmac_network::WifiNetwork>,
    Vec<rmac_network::WifiSavedNetwork>,
    Option<SharedString>,
);

impl Settings {
    /// Read the current radio state when returning to Wi-Fi. Hidden-pane
    /// NetworkManager signals do not run a full snapshot in the background.
    pub(in crate::controller) fn refresh_wifi_state(&mut self, cx: &mut Context<Self>) {
        if self.wifi_loading || self.wifi_busy || self.wifi_scanning {
            return;
        }
        let generation = self.wifi_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_network::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if wifi_stream_snapshot_is_current(
                    generation,
                    this.wifi_generation,
                    this.wifi_busy,
                    this.wifi_loading,
                ) {
                    this.finish_wifi_stream_update(result);
                    this.notify_if_current_pane(&["Wi-Fi"], cx);
                }
            });
        })
        .detach();
    }

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

    /// Everything a Wi-Fi refresh can change that the window shows, to tell
    /// whether a background refresh needs a repaint at all.
    pub(in crate::controller) fn wifi_view_state(&self) -> WifiViewState {
        (
            self.wifi_loading,
            self.wifi_busy,
            self.wifi_available,
            self.wifi_on,
            self.wifi_interface.clone(),
            self.wifi_networks.clone(),
            self.wifi_saved_networks.clone(),
            self.wifi_error.clone(),
        )
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
            // `rmac_network::set_enabled()`/`snapshot()` use
            // `zbus::blocking`; GPUI's background executor is not safe to
            // block on synchronous D-Bus I/O from (LINUX-HW-07).
            let result = blocking::unblock(move || {
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
}
