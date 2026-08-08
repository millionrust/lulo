//! Wi-Fi snapshot, mutation, credential, and recovery lifecycle.

use super::*;

mod render;

impl Settings {
    pub(super) fn apply_wifi_snapshot(&mut self, snapshot: rmac_network::WifiSnapshot) {
        self.wifi_available = snapshot.available;
        self.wifi_on = snapshot.enabled;
        self.wifi_interface = snapshot.interface;
        self.wifi_networks = snapshot.networks;
        self.wifi_saved_networks = snapshot.saved_networks;
    }

    pub(super) fn begin_wifi_mutation(&mut self) {
        self.wifi_generation = self.wifi_generation.wrapping_add(1);
        self.wifi_busy = true;
    }

    pub(super) fn finish_wifi_stream_update(
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

    pub(super) fn finish_wifi_update(
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

    pub(super) fn finish_wifi_password_update(
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

    pub(super) fn finish_wifi_enterprise_update(
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

    pub(super) fn finish_wifi_forget_update(
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

    pub(super) fn set_wifi_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
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

    pub(super) fn refresh_wifi(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn connect_wifi(
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

    pub(super) fn request_wifi_forget(
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

    pub(super) fn cancel_wifi_forget(&mut self, cx: &mut Context<Self>) {
        if !self.wifi_busy {
            self.wifi_forget_confirmation = None;
            cx.notify();
        }
    }

    pub(super) fn confirm_wifi_forget(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn select_wifi_network(
        &mut self,
        network: rmac_network::WifiNetwork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy
            || self.wifi_loading
            || !self.wifi_available
            || !self.wifi_on
            || !network.can_connect()
            || !self
                .wifi_networks
                .iter()
                .any(|candidate| candidate.id == network.id)
        {
            return;
        }
        let action = wifi_join_action(&network);
        if action == WifiJoinAction::EnterpriseSetup {
            let identity = cx.new(|cx| {
                InputState::new(window, cx)
                    .clean_on_escape()
                    .placeholder("name@example.com")
            });
            let certificate_domain = cx.new(|cx| {
                InputState::new(window, cx)
                    .clean_on_escape()
                    .placeholder("radius.example.com")
            });
            let anonymous_identity = cx.new(|cx| {
                InputState::new(window, cx)
                    .clean_on_escape()
                    .placeholder("Optional")
            });
            let password = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .clean_on_escape()
                    .placeholder("Password")
            });
            let focus = identity.read(cx).focus_handle(cx);
            self.wifi_enterprise_prompt = Some(WifiEnterprisePrompt {
                network: network.id,
                ssid: network.ssid.into(),
                identity,
                anonymous_identity,
                certificate_domain,
                password,
                validation_error: None,
            });
            self.wifi_error = None;
            window.focus(&focus);
            cx.notify();
            return;
        }
        if action == WifiJoinAction::Direct {
            self.connect_wifi(network.id, cx);
            return;
        }
        if action != WifiJoinAction::PersonalPassword {
            return;
        }

        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = editor.read(cx).focus_handle(cx);
        self.wifi_password_prompt = Some(WifiPasswordPrompt {
            network: network.id,
            ssid: network.ssid.into(),
            editor,
            validation_error: None,
        });
        self.wifi_error = None;
        window.focus(&focus);
        cx.notify();
    }

    pub(super) fn cancel_wifi_password(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_password_prompt = None;
            self.wifi_connecting = None;
        }
        cx.notify();
    }

    pub(super) fn submit_wifi_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        let Some(prompt) = &self.wifi_password_prompt else {
            return;
        };
        let network = prompt.network.clone();
        let value = prompt.editor.read(cx).value().to_string();
        let password = match rmac_network::WifiPassword::new(value, &network) {
            Ok(password) => password,
            Err(error) => {
                if let Some(prompt) = &mut self.wifi_password_prompt {
                    prompt.validation_error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };

        // Drop the editor entity that contained the secret, including its undo
        // history, as soon as ownership moves into the zeroizing password type.
        let empty_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = empty_editor.read(cx).focus_handle(cx);
        if let Some(prompt) = &mut self.wifi_password_prompt {
            prompt.editor = empty_editor;
            prompt.validation_error = None;
        }
        window.focus(&focus);

        let cancellation = rmac_network::WifiCancellation::new();
        self.begin_wifi_mutation();
        self.wifi_connecting = Some(network.clone());
        self.wifi_cancellation = Some(cancellation.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result =
                        rmac_network::connect_with_password(&network, password, &cancellation);
                    let recovery_snapshot = result
                        .is_err()
                        .then(|| rmac_network::snapshot().ok())
                        .flatten();
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_password_update(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_wifi_enterprise(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_enterprise_prompt = None;
            self.wifi_connecting = None;
        }
        cx.notify();
    }

    pub(super) fn submit_wifi_enterprise(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        let Some(prompt) = &self.wifi_enterprise_prompt else {
            return;
        };
        let network = prompt.network.clone();
        let identity = prompt.identity.read(cx).value().to_string();
        let anonymous_identity = prompt.anonymous_identity.read(cx).value().to_string();
        let certificate_domain = prompt.certificate_domain.read(cx).value().to_string();
        let password = prompt.password.read(cx).value().to_string();
        let credentials = match rmac_network::WifiEnterpriseCredentials::new(
            identity,
            anonymous_identity,
            certificate_domain,
            password,
            &network,
        ) {
            Ok(credentials) => credentials,
            Err(error) => {
                if let Some(prompt) = &mut self.wifi_enterprise_prompt {
                    prompt.validation_error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };

        // The non-secret identity and certificate domain remain available for
        // correction, but discard the password editor and its undo history as
        // soon as the zeroizing credentials take ownership.
        let empty_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = empty_password.read(cx).focus_handle(cx);
        if let Some(prompt) = &mut self.wifi_enterprise_prompt {
            prompt.password = empty_password;
            prompt.validation_error = None;
        }
        window.focus(&focus);

        let cancellation = rmac_network::WifiCancellation::new();
        self.begin_wifi_mutation();
        self.wifi_connecting = Some(network.clone());
        self.wifi_cancellation = Some(cancellation.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result =
                        rmac_network::connect_enterprise(&network, credentials, &cancellation);
                    let recovery_snapshot = result
                        .is_err()
                        .then(|| rmac_network::snapshot().ok())
                        .flatten();
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_enterprise_update(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }
}
