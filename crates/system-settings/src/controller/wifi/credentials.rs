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
        // Windows joins 802.1X networks through its own sign-in prompt,
        // which apps cannot drive: hand the network to Windows' network
        // list, where that prompt lives.
        #[cfg(windows)]
        if action == WifiJoinAction::EnterpriseSetup {
            cx.background_executor()
                .spawn(blocking::unblock(|| {
                    if let Err(error) = std::process::Command::new("explorer.exe")
                        .arg("ms-availablenetworks:")
                        .spawn()
                    {
                        eprintln!("could not open Windows' network list: {error}");
                    }
                }))
                .detach();
            return;
        }
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
            // `rmac_network::connect_with_password()`/`snapshot()` use
            // `zbus::blocking`; GPUI's background executor is not safe to
            // block on synchronous D-Bus I/O from (LINUX-HW-07).
            let (result, recovery_snapshot) = blocking::unblock(move || {
                let result = rmac_network::connect_with_password(&network, password, &cancellation);
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
            // `rmac_network::connect_enterprise()`/`snapshot()` use
            // `zbus::blocking`; GPUI's background executor is not safe to
            // block on synchronous D-Bus I/O from (LINUX-HW-07).
            let (result, recovery_snapshot) = blocking::unblock(move || {
                let result = rmac_network::connect_enterprise(&network, credentials, &cancellation);
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

    /// Wi-Fi ▸ Other…: the Mac's "Find and join a Wi-Fi network" dialog.
    pub(in crate::controller) fn open_wifi_other(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder("Network Name")
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = name.read(cx).focus_handle(cx);
        self.wifi_other_prompt = Some(WifiOtherPrompt {
            name,
            password,
            secure: true,
            validation_error: None,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(in crate::controller) fn set_wifi_other_secure(
        &mut self,
        secure: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = &mut self.wifi_other_prompt {
            prompt.secure = secure;
            prompt.validation_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn cancel_wifi_other(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_other_prompt = None;
        }
        cx.notify();
    }

    pub(in crate::controller) fn submit_wifi_other(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy {
            return;
        }
        let Some(prompt) = &self.wifi_other_prompt else {
            return;
        };
        let name = prompt.name.read(cx).value().to_string();
        let security = if prompt.secure {
            rmac_network::WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk)
        } else {
            rmac_network::WifiSecurity::Open
        };
        let Some(network) = rmac_network::WifiNetworkId::from_bytes(name.into_bytes(), security)
        else {
            if let Some(prompt) = &mut self.wifi_other_prompt {
                prompt.validation_error = Some("Enter a network name of 1 to 32 bytes.".into());
            }
            cx.notify();
            return;
        };
        let password = if prompt.secure {
            let value = prompt.password.read(cx).value().to_string();
            match rmac_network::WifiPassword::new(value, &network) {
                Ok(password) => Some(password),
                Err(error) => {
                    if let Some(prompt) = &mut self.wifi_other_prompt {
                        prompt.validation_error = Some(error.to_string().into());
                    }
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        // Drop the editor that held the secret as soon as it is owned by
        // the zeroizing password type.
        let empty = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        if let Some(prompt) = &mut self.wifi_other_prompt {
            prompt.password = empty;
            prompt.validation_error = None;
        }
        let cancellation = rmac_network::WifiCancellation::new();
        self.begin_wifi_mutation();
        self.wifi_cancellation = Some(cancellation.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // Blocking D-Bus I/O stays off GPUI's executor (LINUX-HW-07).
            let (result, recovery_snapshot) = blocking::unblock(move || {
                let result = rmac_network::join_hidden(&network, password, &cancellation);
                let recovery_snapshot = result
                    .is_err()
                    .then(|| rmac_network::snapshot().ok())
                    .flatten();
                (result, recovery_snapshot)
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_other_update(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_wifi_other_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
        recovery_snapshot: Option<rmac_network::WifiSnapshot>,
    ) {
        self.wifi_busy = false;
        self.wifi_cancellation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_other_prompt = None;
            }
            Err(error) if error.is_cancelled() => {
                self.wifi_other_prompt = None;
            }
            Err(error) => {
                if let Some(snapshot) = recovery_snapshot {
                    self.apply_wifi_snapshot(snapshot);
                }
                if let Some(prompt) = &mut self.wifi_other_prompt {
                    prompt.validation_error =
                        Some(format!("Could not join this network: {error}").into());
                }
            }
        }
    }
}
