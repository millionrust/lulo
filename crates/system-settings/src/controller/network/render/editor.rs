//! Network editor presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_network_editor(&self, cx: &Context<Self>) -> Vec<Div> {
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
