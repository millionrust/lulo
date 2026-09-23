//! Network settings presentation.

use super::*;

mod editor;

impl Settings {
    pub(in crate::controller) fn render_network(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let connected = matches!(
            self.network.connectivity,
            rmac_network::Connectivity::Full
                | rmac_network::Connectivity::Limited
                | rmac_network::Connectivity::Portal
        );
        let summary = self
            .network
            .primary_connection
            .clone()
            .unwrap_or_else(|| "No primary connection".into());
        let refresh_view = view.clone();
        let mut cards = vec![card(vec![
            value_row(
                "icons/globe.svg",
                if connected {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                "Status".into(),
                self.network.connectivity.label().into(),
            ),
            value_row(
                "icons/folder-symlink.svg",
                accent(),
                "Primary Connection".into(),
                summary.into(),
            ),
        ])];
        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(style::ROW_PADDING))
                .pt(px(style::SECTION_TOP))
                .pb(px(style::SECTION_BOTTOM))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(style::heading_text())
                        .child("Interfaces"),
                )
                .child(
                    Button::new("network-refresh", "Refresh")
                        .ghost()
                        .busy(self.network_busy)
                        .disabled(self.network_loading || self.network_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| settings.refresh_network(cx));
                        }),
                ),
        );

        if self.network_loading {
            cards.push(note_card("Loading network state from the system…"));
            return self.pane(cards);
        }
        if !self.network.available {
            cards.push(note_card(
                "The system network service is not available on this computer.",
            ));
            return self.pane(cards);
        }
        if self.network.devices.is_empty() {
            cards.push(note_card("No managed network interfaces were found."));
            return self.pane(cards);
        }

        if self.network_editor.is_some() {
            cards.extend(self.render_network_editor(cx));
        }

        for device in &self.network.devices {
            let title = device
                .connection
                .as_deref()
                .unwrap_or_else(|| device.kind.label());
            let heading = if device.primary {
                format!("{title} · Primary")
            } else {
                title.to_string()
            };
            cards.push(
                div()
                    .px(px(style::ROW_PADDING))
                    .pt(px(style::SECTION_TOP))
                    .pb(px(style::SECTION_BOTTOM))
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(style::heading_text())
                    .child(heading),
            );
            let mut rows = vec![value_row(
                if device.kind == rmac_network::DeviceKind::WiFi {
                    "icons/wifi.svg"
                } else {
                    "icons/globe.svg"
                },
                if device.state.is_connected() {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                format!("{} ({})", device.kind.label(), device.interface).into(),
                device.state.label().into(),
            )];
            if !device.addresses.is_empty() {
                rows.push(value_row(
                    "icons/globe.svg",
                    secondary(),
                    "IP Addresses".into(),
                    device.addresses.join(", ").into(),
                ));
            }
            if let Some(gateway) = &device.gateway {
                rows.push(value_row(
                    "icons/folder-symlink.svg",
                    secondary(),
                    "Router".into(),
                    gateway.clone().into(),
                ));
            }
            if !device.dns.is_empty() {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "DNS Servers".into(),
                    device.dns.join(", ").into(),
                ));
            }
            if let Some(address) = &device.hardware_address {
                rows.push(value_row(
                    "icons/key.svg",
                    secondary(),
                    "Hardware Address".into(),
                    address.clone().into(),
                ));
            }
            if let Some(configuration) = &device.configuration {
                let edit_view = view.clone();
                let edit_interface = device.interface.clone();
                let edit_configuration = configuration.clone();
                let enabled = configuration.editable && !self.network_busy;
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            configuration
                                .limitation
                                .as_ref()
                                .map(|limitation| limitation.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-{}",
                                    device.interface
                                ))),
                                if configuration.editable {
                                    "Details…"
                                } else {
                                    "Read Only"
                                },
                            )
                            .disabled(!enabled)
                            .on_click(move |_, window, cx| {
                                edit_view.update(cx, |settings, cx| {
                                    settings.start_network_edit(
                                        edit_interface.clone(),
                                        edit_configuration.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            }),
                        )
                        .into_any_element(),
                );
            } else if let Some(error) = &device.configuration_error {
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            Some(error.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-unavailable-{}",
                                    device.interface
                                ))),
                                "Unavailable",
                            )
                            .disabled(true),
                        )
                        .into_any_element(),
                );
            } else {
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            Some("Connect this interface to edit its active profile".into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-inactive-{}",
                                    device.interface
                                ))),
                                "Unavailable",
                            )
                            .disabled(true),
                        )
                        .into_any_element(),
                );
            }
            cards.push(card(rows));
        }
        self.pane(cards)
    }
}
