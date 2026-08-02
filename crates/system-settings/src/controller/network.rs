//! Wired/network-details editing and snapshot lifecycle.

use super::*;

impl Settings {
    pub(super) fn start_network_edit(
        &mut self,
        interface: String,
        configuration: rmac_network::NetworkConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.network_busy || self.network_loading || !configuration.editable {
            return;
        }
        let ipv4_addresses = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv4.addresses_text())
                .placeholder("192.0.2.20/24")
        });
        let ipv4_gateway = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv4.gateway_text())
                .placeholder("192.0.2.1")
        });
        let ipv4_dns = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv4.dns_text())
                .placeholder("1.1.1.1, 9.9.9.9")
        });
        let ipv6_addresses = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv6.addresses_text())
                .placeholder("2001:db8::20/64")
        });
        let ipv6_gateway = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv6.gateway_text())
                .placeholder("2001:db8::1")
        });
        let ipv6_dns = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.ipv6.dns_text())
                .placeholder("2606:4700:4700::1111")
        });
        let proxy_url = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(configuration.proxy.pac_url_text().to_owned())
                .placeholder("https://proxy.example/proxy.pac")
        });
        self.network_editor = Some(NetworkEditorState {
            interface: interface.into(),
            ipv4_method: configuration.ipv4.method.clone(),
            ipv4_ignore_auto_dns: configuration.ipv4.ignore_auto_dns,
            ipv6_method: configuration.ipv6.method.clone(),
            ipv6_ignore_auto_dns: configuration.ipv6.ignore_auto_dns,
            proxy_method: configuration.proxy.method,
            proxy_browser_only: configuration.proxy.browser_only,
            configuration,
            ipv4_addresses,
            ipv4_gateway,
            ipv4_dns,
            ipv6_addresses,
            ipv6_gateway,
            ipv6_dns,
            proxy_url,
            validation_error: None,
        });
        self.network_error = None;
        cx.notify();
    }

    pub(super) fn set_network_ip_method(
        &mut self,
        family: rmac_network::IpFamily,
        method: rmac_network::IpMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.network_busy {
            return;
        }
        let Some(editor) = &self.network_editor else {
            return;
        };
        let fields = match family {
            rmac_network::IpFamily::V4 => [
                editor.ipv4_addresses.clone(),
                editor.ipv4_gateway.clone(),
                editor.ipv4_dns.clone(),
            ],
            rmac_network::IpFamily::V6 => [
                editor.ipv6_addresses.clone(),
                editor.ipv6_gateway.clone(),
                editor.ipv6_dns.clone(),
            ],
        };
        if matches!(
            method,
            rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
        ) {
            for field in fields {
                field.update(cx, |state, cx| state.set_value("", window, cx));
            }
        }
        if let Some(editor) = &mut self.network_editor {
            match family {
                rmac_network::IpFamily::V4 => {
                    editor.ipv4_method = method;
                    if matches!(
                        editor.ipv4_method,
                        rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
                    ) {
                        editor.ipv4_ignore_auto_dns = false;
                    }
                }
                rmac_network::IpFamily::V6 => {
                    editor.ipv6_method = method;
                    if matches!(
                        editor.ipv6_method,
                        rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
                    ) {
                        editor.ipv6_ignore_auto_dns = false;
                    }
                }
            }
            editor.validation_error = None;
        }
        cx.notify();
    }

    pub(super) fn set_network_ignore_auto_dns(
        &mut self,
        family: rmac_network::IpFamily,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        if self.network_busy {
            return;
        }
        if let Some(editor) = &mut self.network_editor {
            match family {
                rmac_network::IpFamily::V4 => editor.ipv4_ignore_auto_dns = value,
                rmac_network::IpFamily::V6 => editor.ipv6_ignore_auto_dns = value,
            }
            editor.validation_error = None;
            cx.notify();
        }
    }

    pub(super) fn set_network_proxy_method(
        &mut self,
        method: rmac_network::ProxyMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.network_busy {
            return;
        }
        let Some(editor) = &self.network_editor else {
            return;
        };
        let proxy_url = editor.proxy_url.clone();
        if method == rmac_network::ProxyMethod::None {
            proxy_url.update(cx, |state, cx| state.set_value("", window, cx));
        }
        if let Some(editor) = &mut self.network_editor {
            editor.proxy_method = method;
            if method == rmac_network::ProxyMethod::None {
                editor.proxy_browser_only = false;
            }
            editor.validation_error = None;
        }
        cx.notify();
    }

    pub(super) fn set_network_proxy_browser_only(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.network_busy {
            return;
        }
        if let Some(editor) = &mut self.network_editor {
            editor.proxy_browser_only = value;
            editor.validation_error = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_network_edit(&mut self, cx: &mut Context<Self>) {
        if !self.network_busy {
            self.network_editor = None;
            self.network_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_network_edit(&mut self, cx: &mut Context<Self>) {
        if self.network_busy || self.network_loading {
            return;
        }
        let Some(editor) = &self.network_editor else {
            return;
        };
        let ipv4_addresses = editor.ipv4_addresses.read(cx).value();
        let ipv4_gateway = editor.ipv4_gateway.read(cx).value();
        let ipv4_dns = editor.ipv4_dns.read(cx).value();
        let ipv6_addresses = editor.ipv6_addresses.read(cx).value();
        let ipv6_gateway = editor.ipv6_gateway.read(cx).value();
        let ipv6_dns = editor.ipv6_dns.read(cx).value();
        let proxy_url = editor.proxy_url.read(cx).value();
        let ipv4 = rmac_network::IpConfiguration::parse(
            rmac_network::IpFamily::V4,
            editor.ipv4_method.clone(),
            &ipv4_addresses,
            &ipv4_gateway,
            &ipv4_dns,
            editor.ipv4_ignore_auto_dns,
        );
        let ipv6 = rmac_network::IpConfiguration::parse(
            rmac_network::IpFamily::V6,
            editor.ipv6_method.clone(),
            &ipv6_addresses,
            &ipv6_gateway,
            &ipv6_dns,
            editor.ipv6_ignore_auto_dns,
        );
        let proxy = rmac_network::ProxyConfiguration::new(
            editor.proxy_method,
            &proxy_url,
            editor.proxy_browser_only,
        );
        let edit = match ipv4
            .and_then(|ipv4| ipv6.map(|ipv6| (ipv4, ipv6)))
            .and_then(|(ipv4, ipv6)| proxy.map(|proxy| (ipv4, ipv6, proxy)))
            .and_then(|(ipv4, ipv6, proxy)| {
                rmac_network::NetworkEdit::new(&editor.configuration, ipv4, ipv6, proxy)
            }) {
            Ok(edit) => edit,
            Err(error) => {
                if let Some(editor) = &mut self.network_editor {
                    editor.validation_error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };
        if edit.ipv4 == editor.configuration.ipv4
            && edit.ipv6 == editor.configuration.ipv6
            && edit.proxy == editor.configuration.proxy
        {
            self.network_editor = None;
            cx.notify();
            return;
        }
        self.network_generation = self.network_generation.wrapping_add(1);
        self.network_busy = true;
        self.network_error = None;
        if let Some(editor) = &mut self.network_editor {
            editor.validation_error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::update_network_connection(&edit);
                    let recovery = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::network_snapshot().ok());
                    (result, recovery)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.network_loading = false;
                this.network_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.network = snapshot;
                        this.network_editor = None;
                        this.network_error = None;
                        this.network_stream_error = None;
                    }
                    Err(error) => {
                        if let Some(snapshot) = recovery {
                            this.network = snapshot;
                        }
                        let message: SharedString =
                            format!("Could not save Network settings: {error}").into();
                        this.network_error = Some(message.clone());
                        if let Some(editor) = &mut this.network_editor {
                            editor.validation_error = Some(message);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_network_stream_update(
        &mut self,
        result: std::result::Result<rmac_network::NetworkSnapshot, rmac_network::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                let stale_editor = self.apply_external_network_snapshot(snapshot);
                self.network_stream_error = None;
                if stale_editor {
                    self.network_error = Some(
                        "The connection profile changed outside System Settings. Reopen Details to edit the current values."
                            .into(),
                    );
                }
            }
            Err(_) => {
                self.network_stream_error =
                    Some("Live Network state could not be refreshed from NetworkManager".into());
            }
        }
    }

    pub(super) fn apply_external_network_snapshot(
        &mut self,
        snapshot: rmac_network::NetworkSnapshot,
    ) -> bool {
        let stale_editor = self.network_editor.as_ref().is_some_and(|editor| {
            snapshot
                .devices
                .iter()
                .filter_map(|device| device.configuration.as_ref())
                .find(|configuration| configuration.id == editor.configuration.id)
                != Some(&editor.configuration)
        });
        self.network = snapshot;
        if stale_editor {
            self.network_editor = None;
        }
        stale_editor
    }

    pub(super) fn finish_network_update(
        &mut self,
        result: std::result::Result<rmac_network::NetworkSnapshot, rmac_network::Error>,
    ) {
        self.network_loading = false;
        self.network_busy = false;
        match result {
            Ok(snapshot) => {
                if self.apply_external_network_snapshot(snapshot) {
                    self.network_error = Some(
                        "The connection profile changed outside System Settings. Reopen Details to edit the current values."
                            .into(),
                    );
                } else {
                    self.network_error = None;
                }
                self.network_stream_error = None;
            }
            Err(error) => {
                self.network_error = Some(format!("Could not update Network: {error}").into());
            }
        }
    }

    pub(super) fn refresh_network(&mut self, cx: &mut Context<Self>) {
        if self.network_busy || self.network_loading {
            return;
        }
        self.network_generation = self.network_generation.wrapping_add(1);
        self.network_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::network_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_network_update(result);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_network(&self, cx: &Context<Self>) -> Div {
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
                .px_1()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
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
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
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

    pub(super) fn render_network_editor(&self, cx: &Context<Self>) -> Vec<Div> {
        let Some(editor) = &self.network_editor else {
            return Vec::new();
        };
        let view = cx.entity();
        let busy = self.network_busy;
        let ipv4_values_enabled = !busy
            && !matches!(
                editor.ipv4_method,
                rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
            );
        let ipv6_values_enabled = !busy
            && !matches!(
                editor.ipv6_method,
                rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
            );
        let proxy_values_enabled =
            !busy && editor.proxy_method == rmac_network::ProxyMethod::Automatic;
        let cancel_view = view.clone();
        let save_view = view.clone();
        let mut sections = vec![
            section_header(format!(
                "{} · {}",
                editor.configuration.name, editor.interface
            )),
            note_card(
                "IP and DNS changes are staged on the active connection and verified before the complete profile is saved. Proxy-only changes are saved atomically. On failure, rmac restores the previous authority only when no newer external edit would be overwritten.",
            ),
            section_header("IPv4"),
            card(vec![
                network_ip_method_row(
                    view.clone(),
                    rmac_network::IpFamily::V4,
                    &editor.ipv4_method,
                    !busy,
                ),
                network_field_row(
                    "Addresses",
                    "Comma-separated addresses with prefixes",
                    &editor.ipv4_addresses,
                    ipv4_values_enabled,
                ),
                network_field_row(
                    "Router",
                    "Optional default gateway",
                    &editor.ipv4_gateway,
                    ipv4_values_enabled,
                ),
                network_field_row(
                    "DNS Servers",
                    "Comma-separated IPv4 addresses",
                    &editor.ipv4_dns,
                    ipv4_values_enabled,
                ),
                network_dns_policy_row(
                    view.clone(),
                    rmac_network::IpFamily::V4,
                    editor.ipv4_ignore_auto_dns,
                    ipv4_values_enabled,
                ),
            ]),
            section_header("IPv6"),
            card(vec![
                network_ip_method_row(
                    view.clone(),
                    rmac_network::IpFamily::V6,
                    &editor.ipv6_method,
                    !busy,
                ),
                network_field_row(
                    "Addresses",
                    "Comma-separated addresses with prefixes",
                    &editor.ipv6_addresses,
                    ipv6_values_enabled,
                ),
                network_field_row(
                    "Router",
                    "Optional default gateway",
                    &editor.ipv6_gateway,
                    ipv6_values_enabled,
                ),
                network_field_row(
                    "DNS Servers",
                    "Comma-separated IPv6 addresses",
                    &editor.ipv6_dns,
                    ipv6_values_enabled,
                ),
                network_dns_policy_row(
                    view.clone(),
                    rmac_network::IpFamily::V6,
                    editor.ipv6_ignore_auto_dns,
                    ipv6_values_enabled,
                ),
            ]),
            section_header("Proxy"),
            card(vec![
                network_proxy_method_row(view.clone(), editor.proxy_method, !busy),
                network_field_row(
                    "Configuration URL",
                    "HTTP, HTTPS, or absolute file URL",
                    &editor.proxy_url,
                    proxy_values_enabled,
                ),
                network_proxy_browser_row(
                    view.clone(),
                    editor.proxy_browser_only,
                    proxy_values_enabled,
                ),
            ]),
        ];
        if let Some(error) = &editor.validation_error {
            sections.push(note_card(error.clone()));
        }
        sections.push(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .mb_3()
                .child(
                    Button::new("network-edit-cancel", "Cancel")
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_network_edit(cx));
                        }),
                )
                .child(
                    Button::new(
                        "network-edit-save",
                        if busy { "Applying…" } else { "Apply" },
                    )
                    .primary()
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |settings, cx| settings.submit_network_edit(cx));
                    }),
                ),
        );
        sections
    }
}
