//! System Settings Network editor IP, DNS, and proxy control-row projection.

use super::*;

pub(in crate::controller) fn network_ip_method_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    selected: &rmac_network::IpMethod,
    enabled: bool,
) -> AnyElement {
    let options = match family {
        rmac_network::IpFamily::V4 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
        rmac_network::IpFamily::V6 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("DHCP", rmac_network::IpMethod::Dhcp),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
    };
    let choices: Vec<PopupChoice> = options
        .into_iter()
        .map(|(label, method)| {
            let method_view = view.clone();
            let checked = *selected == method;
            choice(label, checked, move |window, cx| {
                let method = method.clone();
                method_view.update(cx, |settings, cx| {
                    settings.set_network_ip_method(family, method, window, cx)
                });
            })
        })
        .collect();
    let current = popup_value(&choices, "Custom");
    popup_row(
        SharedString::from(format!(
            "network-{}-method",
            match family {
                rmac_network::IpFamily::V4 => "ipv4",
                rmac_network::IpFamily::V6 => "ipv6",
            }
        )),
        "Configure",
        Some("Choose how this connection receives addresses".into()),
        current,
        choices,
        enabled,
    )
}

pub(in crate::controller) fn network_field_row(
    title: &'static str,
    subtitle: &'static str,
    editor: &Entity<InputState>,
    enabled: bool,
) -> AnyElement {
    row_base()
        .child(text_block(title.into(), Some(subtitle.into())))
        .child(
            div()
                .w(px(390.0))
                .child(TextField::new(editor).small().disabled(!enabled)),
        )
        .into_any_element()
}

pub(in crate::controller) fn network_dns_policy_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let id = match family {
        rmac_network::IpFamily::V4 => "network-ipv4-ignore-auto-dns",
        rmac_network::IpFamily::V6 => "network-ipv6-ignore-auto-dns",
    };
    row_base()
        .child(text_block(
            "Use only these DNS servers".into(),
            Some("Ignore DNS supplied automatically by the network".into()),
        ))
        .child(
            Toggle::new(id)
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_ignore_auto_dns(family, *value, cx)
                    });
                }),
        )
        .into_any_element()
}

pub(in crate::controller) fn network_proxy_method_row(
    view: Entity<Settings>,
    selected: rmac_network::ProxyMethod,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = [
        ("Off", rmac_network::ProxyMethod::None),
        ("Automatic", rmac_network::ProxyMethod::Automatic),
    ]
    .into_iter()
    .map(|(label, method)| {
        let method_view = view.clone();
        choice(label, selected == method, move |window, cx| {
            method_view.update(cx, |settings, cx| {
                settings.set_network_proxy_method(method, window, cx)
            });
        })
    })
    .collect();
    let current = popup_value(&choices, "Off");
    popup_row(
        "network-proxy-method",
        "Configure",
        Some("Use a proxy auto-configuration source".into()),
        current,
        choices,
        enabled,
    )
}

pub(in crate::controller) fn network_proxy_browser_row(
    view: Entity<Settings>,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    row_base()
        .child(text_block(
            "Web browsers only".into(),
            Some("Other applications may ignore this proxy configuration".into()),
        ))
        .child(
            Toggle::new("network-proxy-browser-only")
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_proxy_browser_only(*value, cx)
                    });
                }),
        )
        .into_any_element()
}
