//! Wi-Fi snapshot, mutation, credential, and recovery lifecycle.

use super::*;

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

    pub(super) fn render_wifi_forget_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_forget_confirmation.as_ref()?;
        let message = format!(
            "This computer will remove every accessible saved profile for “{}”. If the network is active, it will disconnect. You will need its password to join again.",
            prompt.ssid
        );
        Some(
            rmac_ui::alert(
                "Forget This Network?",
                message,
                vec![
                    rmac_ui::dialog_button(
                        "wifi-forget-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_forget(cx)))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "wifi-forget-confirm",
                        "Forget",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_wifi_forget(cx)))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }
    pub(super) fn render_wifi(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let power_subtitle = if self.wifi_loading {
            Some("Reading system state…".into())
        } else if self.wifi_forgetting.is_some() {
            Some("Forgetting saved network…".into())
        } else if self.wifi_connecting.is_some() {
            Some("Connecting to network…".into())
        } else if self.wifi_busy {
            Some("Applying change…".into())
        } else {
            self.wifi_interface
                .as_ref()
                .map(|interface| format!("NetworkManager · {interface}").into())
        };
        let power_view = view.clone();
        let power = Toggle::new("wifi-power")
            .checked(self.wifi_on)
            .disabled(self.wifi_loading || self.wifi_busy || !self.wifi_available)
            .on_click(move |enabled, _, cx| {
                power_view.update(cx, |settings, cx| settings.set_wifi_enabled(*enabled, cx));
            });
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/wifi.svg", accent(), 22.0))
            .child(text_block("Wi-Fi".into(), power_subtitle))
            .child(power)
            .into_any_element()])];

        if self.wifi_loading {
            cards.push(note_card("Loading Wi-Fi state from the system…"));
            return self.pane(cards);
        }
        if !self.wifi_available {
            cards.push(note_card(
                "No Wi-Fi adapter is available through the system network service.",
            ));
            return self.pane(cards);
        }

        if self.wifi_on {
            let refresh_view = view.clone();
            let refresh = Button::new("wifi-refresh", "Refresh")
                .small()
                .ghost()
                .busy(self.wifi_busy)
                .disabled(self.wifi_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_wifi(cx));
                });
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
                            .child("Networks"),
                    )
                    .child(refresh),
            );

            let rows = if self.wifi_networks.is_empty() {
                vec![EmptyState::new("No networks found")
                    .message("Refresh to scan again")
                    .into_any_element()]
            } else {
                self.wifi_networks
                    .iter()
                    .enumerate()
                    .map(|(index, network)| {
                        let connecting = self.wifi_connecting.as_ref() == Some(&network.id);
                        let forgetting = self.wifi_forgetting.as_ref() == Some(&network.id);
                        let status = if forgetting {
                            "Forgetting…".to_string()
                        } else if connecting {
                            "Connecting…".to_string()
                        } else if network.connected {
                            format!("Connected · {}%", network.strength)
                        } else if network.known {
                            format!("Known Network · {}%", network.strength)
                        } else {
                            let security = match network.security {
                                rmac_network::WifiSecurity::Open => "Open Network",
                                rmac_network::WifiSecurity::EnhancedOpen => "Enhanced Open",
                                rmac_network::WifiSecurity::Personal(_) => "Password Required",
                                rmac_network::WifiSecurity::Enterprise => {
                                    "Enterprise Setup Required"
                                }
                                rmac_network::WifiSecurity::Legacy => "Unsupported Legacy Security",
                                rmac_network::WifiSecurity::Protected => "Unsupported Security",
                            };
                            format!("{security} · {}%", network.strength)
                        };
                        let can_connect = !self.wifi_busy && network.can_connect();
                        let selected_network = network.clone();
                        let network_view = view.clone();
                        ListRow::new(
                            SharedString::from(format!("wifi-network-{index}")),
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(tile(
                                    "icons/wifi.svg",
                                    if network.connected || connecting || forgetting {
                                        accent()
                                    } else {
                                        secondary()
                                    },
                                    22.0,
                                ))
                                .child(text_block(network.ssid.clone().into(), None))
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .text_color(secondary())
                                        .child(status),
                                ),
                        )
                        .h(px(50.0))
                        .px_3()
                        .disabled(!can_connect)
                        .on_activate(move |_, window, cx| {
                            network_view.update(cx, |settings, cx| {
                                settings.select_wifi_network(selected_network.clone(), window, cx)
                            });
                        })
                        .into_any_element()
                    })
                    .collect()
            };
            cards.push(card(rows));
            cards.push(note_card(
                "Select a network to connect. New WPA Personal and SAE networks ask for their password. Enterprise setup supports certificate-verified PEAP with MSCHAPv2; certificate, smart-card, and other EAP methods remain unavailable. Legacy security is not supported.",
            ));
        }

        if !self.wifi_saved_networks.is_empty() {
            cards.push(section_header("Known Networks"));
            let rows = self
                .wifi_saved_networks
                .iter()
                .enumerate()
                .map(|(index, network)| {
                    let forgetting = self.wifi_forgetting.as_ref() == Some(&network.id);
                    let security = match network.id.security() {
                        rmac_network::WifiSecurity::Open => "Saved Open Network",
                        rmac_network::WifiSecurity::EnhancedOpen => "Saved Enhanced Open Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Psk,
                        ) => "Saved WPA Personal Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Sae,
                        ) => "Saved SAE Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Transition,
                        ) => "Saved WPA/SAE Network",
                        rmac_network::WifiSecurity::Enterprise => "Saved Enterprise Network",
                        rmac_network::WifiSecurity::Legacy => "Saved Legacy Network",
                        rmac_network::WifiSecurity::Protected => "Saved Protected Network",
                    };
                    let forget_network = network.id.clone();
                    let forget_ssid = SharedString::from(network.ssid.clone());
                    let forget_view = view.clone();
                    row_base()
                        .child(tile(
                            "icons/wifi.svg",
                            if forgetting { accent() } else { secondary() },
                            22.0,
                        ))
                        .child(text_block(
                            network.ssid.clone().into(),
                            Some(security.into()),
                        ))
                        .child(
                            Button::new(
                                SharedString::from(format!("wifi-saved-forget-{index}")),
                                "Forget…",
                            )
                            .xsmall()
                            .busy(forgetting)
                            .disabled(self.wifi_busy)
                            .on_click(move |_, _, cx| {
                                forget_view.update(cx, |settings, cx| {
                                    settings.request_wifi_forget(
                                        forget_network.clone(),
                                        forget_ssid.clone(),
                                        cx,
                                    )
                                });
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
            cards.push(note_card(
                "Forgetting removes every accessible saved profile with the exact network identity. If it is active, this computer disconnects first.",
            ));
        }

        self.pane(cards)
    }

    pub(super) fn render_wifi_password_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_password_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };

        let content = div()
            .w(px(380.0))
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
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enter the password for this Wi-Fi network."),
                    ),
            )
            .child(TextField::new(&prompt.editor).disabled(busy).w_full())
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Connecting securely…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-password-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_password(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-password-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_password(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-password-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_password(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_password(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_wifi_enterprise_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_enterprise_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };
        let field = |label_text: &'static str, editor: &Entity<InputState>| {
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child(label_text),
                )
                .child(TextField::new(editor).disabled(busy).w_full())
        };

        let content = div()
            .w(px(430.0))
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
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enterprise Wi-Fi · PEAP with MSCHAPv2"),
                    ),
            )
            .child(field("Identity", &prompt.identity))
            .child(field(
                "Anonymous outer identity (optional)",
                &prompt.anonymous_identity,
            ))
            .child(field(
                "Server certificate domain",
                &prompt.certificate_domain,
            ))
            .child(field("Password", &prompt.password))
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child(
                        "The authentication server must chain to the system trust store and its certificate must match this domain. An anonymous outer identity can avoid exposing the login identity before the secure tunnel forms. Ask your network administrator for both values.",
                    ),
            )
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Verifying and connecting…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-enterprise-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_enterprise(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-enterprise-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_enterprise(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-enterprise-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_enterprise(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_enterprise(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}
