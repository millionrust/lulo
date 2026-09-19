//! Wi-Fi personal and enterprise credential lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn select_wifi_network(
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
            window.focus(&focus, cx);
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
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(in crate::controller) fn cancel_wifi_password(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_password_prompt = None;
            self.wifi_connecting = None;
        }
        cx.notify();
    }

    pub(in crate::controller) fn submit_wifi_password(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        window.focus(&focus, cx);

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

    pub(in crate::controller) fn cancel_wifi_enterprise(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_enterprise_prompt = None;
            self.wifi_connecting = None;
        }
        cx.notify();
    }

    pub(in crate::controller) fn submit_wifi_enterprise(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        window.focus(&focus, cx);

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
