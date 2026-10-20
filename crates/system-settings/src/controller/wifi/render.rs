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
                let details_view = view.clone();
                let interface = self.wifi_interface.clone();
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
                        .when_some(interface, |row, interface| {
                            row.child(push_button("wifi-details", "Details…").on_click(
                                move |_, _, cx| {
                                    let interface = interface.clone();
                                    details_view.update(cx, |settings, cx| {
                                        settings.push(SubPage::NetworkService { interface }, cx)
                                    });
                                },
                            ))
                        })
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
        // A background Wi-Fi watcher hiccup (NetworkManager dropping off
        // the bus) degrades quietly here rather than as the window-wide
        // Settings banner (`global_settings_error`), and clears itself the
        // moment the watcher recovers.
        if let Some(detail) = &self.wifi_stream_error {
            cards.push(note_card(detail.clone()));
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
                    let auto_join = network.auto_join;
                    let auto_join_network = network.id.clone();
                    let auto_join_view = view.clone();
                    let details_view = view.clone();
                    let details_interface =
                        connected.then(|| self.wifi_interface.clone()).flatten();
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
                        // The Mac's circled ⋯: Forget This Network… and, for
                        // the joined network, Network Settings….
                        .child(
                            PopUpButton::new(
                                SharedString::from(format!("wifi-saved-more-{index}")),
                                "Show Detail",
                            )
                            .menu_button("icons/ellipsis.svg", 15.0)
                            .disabled(self.wifi_busy || forgetting)
                            .dropdown_menu(move |menu, _, _| {
                                let forget_view = forget_view.clone();
                                let forget_network = forget_network.clone();
                                let forget_ssid = forget_ssid.clone();
                                let mut menu = menu;
                                if let Some(on) = auto_join {
                                    let auto_join_view = auto_join_view.clone();
                                    let auto_join_network = auto_join_network.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new("Auto-Join").checked(on).on_click(
                                            move |_, _, cx| {
                                                auto_join_view.update(cx, |settings, cx| {
                                                    settings.set_wifi_auto_join(
                                                        auto_join_network.clone(),
                                                        !on,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    );
                                }
                                let mut menu =
                                    menu.item(PopupMenuItem::new("Forget This Network…").on_click(
                                        move |_, _, cx| {
                                            forget_view.update(cx, |settings, cx| {
                                                settings.request_wifi_forget(
                                                    forget_network.clone(),
                                                    forget_ssid.clone(),
                                                    cx,
                                                )
                                            });
                                        },
                                    ));
                                if let Some(interface) = details_interface.clone() {
                                    let details_view = details_view.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new("Network Settings…").on_click(
                                            move |_, _, cx| {
                                                let interface = interface.clone();
                                                details_view.update(cx, |settings, cx| {
                                                    settings.push(
                                                        SubPage::NetworkService { interface },
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }

        if self.wifi_on {
            let scanning = self.wifi_scanning;
            cards.push(
                section_header("Other Networks")
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(scanning, |header| {
                        header.child(
                            div()
                                .id("wifi-scanning-indicator")
                                .role(Role::ProgressIndicator)
                                .aria_label("Scanning for Wi-Fi networks")
                                .child(Spinner::small()),
                        )
                    }),
            );
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
                            .child(if self.wifi_busy || self.wifi_scanning {
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
                        // `ListRow` has no name of its own (see its
                        // `aria_label` doc comment), so without this a
                        // screen reader announced nothing at all for any
                        // network in this list.
                        let aria_label = match &status {
                            Some(status) if !status.is_empty() => {
                                SharedString::from(format!("{}, {status}", network.ssid))
                            }
                            _ => network.ssid.clone().into(),
                        };
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
                        .aria_label(aria_label)
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
            // Other… joins a network that does not broadcast its name
            // (NetworkManager on Lulo OS).
            if cfg!(target_os = "linux") {
                let other_view = view.clone();
                cards.push(footer_buttons(vec![push_button("wifi-other", "Other…")
                    .disabled(self.wifi_busy)
                    .on_click(move |_, window, cx| {
                        other_view.update(cx, |settings, cx| settings.open_wifi_other(window, cx));
                    })
                    .into_any_element()]));
            }
            // Ask to join networks: the menu bar posts the notice
            // (Lulo OS).
            if cfg!(target_os = "linux") {
                let current = self.wifi_ask_to_join;
                let choices = rmac_network::ask_to_join::AskToJoin::ALL
                    .into_iter()
                    .map(|value| {
                        let ask_view = view.clone();
                        choice(value.label(), value == current, move |_, cx| {
                            ask_view.update(cx, |settings, cx| {
                                settings.set_wifi_ask_to_join(value, cx)
                            });
                        })
                    })
                    .collect();
                cards.push(card(vec![popup_row(
                    "wifi-ask-to-join",
                    "Ask to join networks",
                    Some(current.description().into()),
                    current.label().into(),
                    choices,
                    true,
                )]));
            }
            // Advanced…: the known networks with Auto-Join, as on the Mac.
            let advanced_view = view.clone();
            cards.push(footer_buttons(vec![push_button(
                "wifi-advanced",
                "Advanced…",
            )
            .disabled(self.wifi_saved_networks.is_empty())
            .on_click(move |_, _, cx| {
                advanced_view.update(cx, |settings, cx| {
                    settings.set_wifi_advanced_sheet(true, cx)
                });
            })
            .into_any_element()]));
            cards.push(footnote(if cfg!(windows) {
                "New WPA Personal and SAE networks ask for their password. Enterprise networks open Windows' network list, which asks for their sign-in."
            } else {
                "New WPA Personal and SAE networks ask for their password. Enterprise setup supports certificate-verified PEAP with MSCHAPv2; other EAP methods and legacy security are not supported."
            }));
        }

        self.pane(cards)
    }
}
