//! Wired/network-details editing and snapshot lifecycle.

use super::*;

mod render;

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
}
