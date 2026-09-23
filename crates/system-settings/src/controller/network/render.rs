//! Network settings presentation: the macOS 26 service list (52 pt icon rows
//! with a status dot) and one page per service (design-lab/settings.html).

use super::*;

mod editor;

/// The Mac's service icon: Wi-Fi blue, everything else grey.
fn service_icon(device: &rmac_network::NetworkDevice) -> AnyElement {
    if device.kind == rmac_network::DeviceKind::WiFi {
        tile26("icons/wifi.svg", hsl(0x1372f9))
    } else {
        tile26("icons/globe.svg", hsl(0x8e8e93))
    }
}

/// Green when connected, grey while changing, red when not connected.
fn service_status(device: &rmac_network::NetworkDevice) -> AnyElement {
    use rmac_network::DeviceState;
    let color = match device.state {
        DeviceState::Connected => style::status_connected(),
        DeviceState::Connecting | DeviceState::Deactivating | DeviceState::Unknown => {
            style::status_inactive()
        }
        DeviceState::Unavailable | DeviceState::Disconnected | DeviceState::Failed => {
            style::status_disconnected()
        }
    };
    status_line(color, device.state.label())
}

/// The service's name: its connection profile, else its kind, with the
/// interface added when two services would otherwise read the same.
fn service_name(
    device: &rmac_network::NetworkDevice,
    devices: &[rmac_network::NetworkDevice],
) -> String {
    let base = |device: &rmac_network::NetworkDevice| {
        device
            .connection
            .clone()
            .unwrap_or_else(|| device.kind.label().to_string())
    };
    let name = base(device);
    let shared = devices
        .iter()
        .filter(|other| other.interface != device.interface && base(other) == name)
        .count();
    if shared > 0 {
        format!("{name} ({})", device.interface)
    } else {
        name
    }
}

impl Settings {
    fn network_refresh_footer(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        footer_buttons(vec![push_button("network-refresh", "Refresh")
            .busy(self.network_busy)
            .disabled(self.network_loading || self.network_busy)
            .on_click(move |_, _, cx| {
                view.update(cx, |settings, cx| settings.refresh_network(cx));
            })
            .into_any_element()])
    }

    /// Returns the loading / unavailable note when the pane has no services
    /// to show.
    fn network_unavailable_note(&self) -> Option<Div> {
        if self.network_loading {
            Some(note_card("Loading network state from the system…"))
        } else if !self.network.available {
            Some(note_card(
                "The system network service is not available on this computer.",
            ))
        } else if self.network.devices.is_empty() {
            Some(note_card("No managed network interfaces were found."))
        } else {
            None
        }
    }

    pub(in crate::controller) fn render_network(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if let Some(note) = self.network_unavailable_note() {
            cards.push(note);
            cards.push(self.network_refresh_footer(cx));
            return self.pane(cards);
        }
        let devices = &self.network.devices;
        let row = |index: usize, device: &rmac_network::NetworkDevice| {
            let interface = device.interface.clone();
            let row_view = view.clone();
            large_nav_row(
                ("network-service", index),
                service_icon(device),
                service_name(device, devices),
                Some(service_status(device)),
                None,
                move |_, cx| {
                    let interface = interface.clone();
                    row_view.update(cx, |settings, cx| {
                        settings.push(SubPage::NetworkService { interface }, cx)
                    });
                },
            )
        };
        // macOS lists each active service in its own group at the top and
        // the rest under "Other Services".
        let (active, other): (Vec<_>, Vec<_>) = devices
            .iter()
            .enumerate()
            .partition(|(_, device)| device.state.is_connected());
        for (index, device) in active {
            cards.push(group().child(row(index, device)));
        }
        if !other.is_empty() {
            cards.push(section_header("Other Services"));
            cards.push(card(
                other
                    .into_iter()
                    .map(|(index, device)| row(index, device))
                    .collect(),
            ));
        }
        cards.push(self.network_refresh_footer(cx));
        self.pane(cards)
    }

    /// One service's page: its header row with Details…, then its facts,
    /// then the editor while it is open for this interface.
    pub(in crate::controller) fn network_service_body(
        &self,
        interface: &str,
        cx: &Context<Self>,
    ) -> Div {
        let view = cx.entity();
        let mut body = div().v_flex();
        if let Some(note) = self.network_unavailable_note() {
            return body.child(note);
        }
        let Some(device) = self
            .network
            .devices
            .iter()
            .find(|device| device.interface == interface)
        else {
            return body.child(note_card("This network service is no longer available."));
        };

        let details = if let Some(configuration) = &device.configuration {
            let edit_view = view.clone();
            let edit_interface = device.interface.clone();
            let edit_configuration = configuration.clone();
            let enabled =
                configuration.editable && !self.network_busy && self.network_editor.is_none();
            push_button(
                SharedString::from(format!("network-edit-{}", device.interface)),
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
            })
            .into_any_element()
        } else {
            push_button(
                SharedString::from(format!("network-edit-unavailable-{}", device.interface)),
                "Details…",
            )
            .disabled(true)
            .into_any_element()
        };
        body = body.child(
            group().child(
                large_row(
                    service_icon(device),
                    service_name(device, &self.network.devices),
                    Some(service_status(device)),
                )
                .child(details),
            ),
        );
        // Why Details… is unavailable, as the old Connection Details row said.
        if let Some(limitation) = device
            .configuration
            .as_ref()
            .and_then(|configuration| configuration.limitation.clone())
        {
            body = body.child(footnote(limitation));
        } else if let Some(error) = &device.configuration_error {
            body = body.child(footnote(error.clone()));
        } else if device.configuration.is_none() {
            body = body.child(footnote("Connect this service to edit its active profile."));
        }

        let mut facts = Vec::new();
        if !device.addresses.is_empty() {
            facts.push(fact_row("IP Address", device.addresses.join(", ")));
        }
        if let Some(gateway) = &device.gateway {
            facts.push(fact_row("Router", gateway.clone()));
        }
        if !device.dns.is_empty() {
            facts.push(fact_row("DNS Servers", device.dns.join(", ")));
        }
        if let Some(address) = &device.hardware_address {
            facts.push(fact_row("Hardware Address", address.clone()));
        }
        if !facts.is_empty() {
            body = body.child(card(facts));
        }

        if self
            .network_editor
            .as_ref()
            .is_some_and(|editor| editor.interface.as_ref() == interface)
        {
            body = body.children(self.render_network_editor(cx));
        }
        body.child(self.network_refresh_footer(cx))
    }
}
