//! Wi-Fi pane presentation, laid out like macOS 26 (design-lab/settings.html):
//! a header card with the Wi-Fi switch and the current network, then Known
//! Networks and the other networks in range.

use super::*;

mod dialogs;

/// Network rows on the Mac are 45 pt apart: 44 tall plus the separator.
const NETWORK_ROW_HEIGHT: f32 = 44.0;

fn security_is_open(security: &rmac_network::WifiSecurity) -> bool {
    matches!(
        security,
        rmac_network::WifiSecurity::Open | rmac_network::WifiSecurity::EnhancedOpen
    )
}

/// The trailing lock and signal glyphs of a network row. Signal strength
/// dims the glyph, as the Mac greys out the missing arcs.
fn network_glyphs(secured: bool, strength: Option<u8>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .when(secured, |glyphs| {
            glyphs.child(glyph("icons/lock.svg", 12.0, label()))
        })
        .when_some(strength, |glyphs, strength| {
            let alpha = 0.35 + 0.65 * f32::from(strength.min(100)) / 100.0;
            glyphs.child(glyph("icons/wifi.svg", 15.0, label().opacity(alpha)))
        })
}

impl Settings {
    pub(in crate::controller) fn render_wifi(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let status = if self.wifi_loading {
            Some("Reading system state…")
        } else if self.wifi_forgetting.is_some() {
            Some("Forgetting saved network…")
        } else if self.wifi_connecting.is_some() {
            Some("Connecting to network…")
        } else if self.wifi_busy {
            Some("Applying change…")
        } else {
            None
        };
        let description: SharedString = match (status, self.wifi_interface.as_ref()) {
            (Some(status), _) => status.into(),
            (None, Some(interface)) => format!(
                "Set up Wi-Fi to wirelessly connect this computer to the internet. Turn on Wi-Fi, then choose a network to join. Adapter: {interface}."
            )
            .into(),
            (None, None) => "Set up Wi-Fi to wirelessly connect this computer to the internet. Turn on Wi-Fi, then choose a network to join.".into(),
        };
        let power_view = view.clone();
        let power = Toggle::new("wifi-power")
            .checked(self.wifi_on)
            .disabled(self.wifi_loading || self.wifi_busy || !self.wifi_available)
            .on_click(move |enabled, _, cx| {
                power_view.update(cx, |settings, cx| settings.set_wifi_enabled(*enabled, cx));
            });
        let mut header_rows = vec![row_base()
            .items_start()
            .gap(px(12.0))
            .child(tile("icons/wifi.svg", accent(), style::HEADER_ICON))
            .child(text_block("Wi-Fi".into(), Some(description)))
            .child(power)
            .into_any_element()];
        if self.wifi_on {
            if let Some(network) = self.wifi_networks.iter().find(|network| network.connected) {
                header_rows.push(
                    row_base()
                        .child(text_block(
                            network.ssid.clone().into(),
                            Some("Connected".into()),
                        ))
                        .child(network_glyphs(
                            !security_is_open(&network.security),
                            Some(network.strength),
                        ))
                        .into_any_element(),
                );
            }
        }
        let mut cards = vec![card(header_rows)];

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

        if !self.wifi_saved_networks.is_empty() {
            cards.push(section_header("Known Networks"));
            let rows = self
                .wifi_saved_networks
                .iter()
                .enumerate()
                .map(|(index, network)| {
                    let forgetting = self.wifi_forgetting.as_ref() == Some(&network.id);
                    let connected = self
                        .wifi_networks
                        .iter()
                        .any(|scanned| scanned.connected && scanned.id == network.id);
                    let forget_network = network.id.clone();
                    let forget_ssid = SharedString::from(network.ssid.clone());
                    let forget_view = view.clone();
                    row_base()
                        .min_h(px(NETWORK_ROW_HEIGHT))
                        .child(
                            div()
                                .w(px(style::ROW_ICON))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .when(connected, |mark| {
                                    mark.child(glyph("icons/check.svg", 13.0, label()))
                                }),
                        )
                        .child(text_block(network.ssid.clone().into(), None))
                        .child(network_glyphs(
                            !security_is_open(&network.id.security()),
                            None,
                        ))
                        .child(
                            push_button(
                                SharedString::from(format!("wifi-saved-forget-{index}")),
                                "Forget…",
                            )
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
        }

        if self.wifi_on {
            cards.push(section_header("Other Networks"));
            let others: Vec<(usize, &rmac_network::WifiNetwork)> = self
                .wifi_networks
                .iter()
                .enumerate()
                .filter(|(_, network)| !network.connected)
                .collect();
            let rows = if others.is_empty() {
                vec![row_base()
                    .justify_center()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(secondary())
                            .child(if self.wifi_busy {
                                "Searching…"
                            } else {
                                "No other networks found"
                            }),
                    )
                    .into_any_element()]
            } else {
                others
                    .into_iter()
                    .map(|(index, network)| {
                        let connecting = self.wifi_connecting.as_ref() == Some(&network.id);
                        let status: Option<SharedString> = if connecting {
                            Some("Connecting…".into())
                        } else if network.known {
                            Some("Known Network".into())
                        } else {
                            match network.security {
                                rmac_network::WifiSecurity::Enterprise => {
                                    Some("Enterprise setup required".into())
                                }
                                rmac_network::WifiSecurity::Legacy => {
                                    Some("Unsupported legacy security".into())
                                }
                                rmac_network::WifiSecurity::Protected => {
                                    Some("Unsupported security".into())
                                }
                                _ => None,
                            }
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
                                .gap(px(8.0))
                                .child(div().w(px(style::ROW_ICON)).flex_none())
                                .child(text_block(network.ssid.clone().into(), status))
                                .child(network_glyphs(
                                    !security_is_open(&network.security),
                                    Some(network.strength),
                                )),
                        )
                        .selected(true)
                        .bg(gpui::transparent_black())
                        .rounded(px(0.0))
                        .h(px(NETWORK_ROW_HEIGHT))
                        .px(px(style::ROW_PADDING))
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
            let refresh_view = view.clone();
            cards.push(footer_buttons(vec![push_button("wifi-refresh", "Refresh")
                .busy(self.wifi_busy)
                .disabled(self.wifi_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_wifi(cx));
                })
                .into_any_element()]));
            cards.push(footnote(
                "New WPA Personal and SAE networks ask for their password. Enterprise setup supports certificate-verified PEAP with MSCHAPv2; other EAP methods and legacy security are not supported.",
            ));
        }

        self.pane(cards)
    }
}
