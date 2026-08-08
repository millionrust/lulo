//! Wi-Fi connection and reviewed-forget lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn connect_wifi(
        &mut self,
        network: rmac_network::WifiNetworkId,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy || self.wifi_loading || !self.wifi_available || !self.wifi_on {
            return;
        }
        let Some(candidate) = self
            .wifi_networks
            .iter()
            .find(|candidate| candidate.id == network)
        else {
            return;
        };
        if !candidate.can_connect() {
            return;
        }
        self.begin_wifi_mutation();
        self.wifi_connecting = Some(network.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::connect(&network) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn request_wifi_forget(
        &mut self,
        network: rmac_network::WifiNetworkId,
        ssid: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy
            || self.wifi_loading
            || !self
                .wifi_saved_networks
                .iter()
                .any(|candidate| candidate.id == network)
        {
            return;
        }
        self.wifi_forget_confirmation = Some(WifiForgetPrompt { network, ssid });
        self.wifi_error = None;
        cx.notify();
    }

    pub(in crate::controller) fn cancel_wifi_forget(&mut self, cx: &mut Context<Self>) {
        if !self.wifi_busy {
            self.wifi_forget_confirmation = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn confirm_wifi_forget(&mut self, cx: &mut Context<Self>) {
        if self.wifi_busy || self.wifi_loading {
            return;
        }
        let Some(network) = self
            .wifi_forget_confirmation
            .as_ref()
            .map(|prompt| prompt.network.clone())
        else {
            return;
        };
        if !self
            .wifi_saved_networks
            .iter()
            .any(|candidate| candidate.id == network)
        {
            self.wifi_forget_confirmation = None;
            self.wifi_error = Some("The saved Wi-Fi network is no longer available.".into());
            cx.notify();
            return;
        }

        self.begin_wifi_mutation();
        self.wifi_forgetting = Some(network.clone());
        self.wifi_forget_confirmation = None;
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::forget(&network);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_forget_update(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }
}
