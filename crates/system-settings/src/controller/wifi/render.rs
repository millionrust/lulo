//! Wi-Fi pane presentation.

use super::*;

mod dialogs;

impl Settings {
    pub(in crate::controller) fn render_wifi(&self, cx: &Context<Self>) -> Div {
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
}
