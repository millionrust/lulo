//! System Settings Sharing pane: the macOS 26 "Content & Media" and
//! "Advanced" groups of 50 pt service rows with a switch and a circled (i)
//! (design-lab/settings.html). rmac backs File Sharing (Samba) and Remote
//! Login (OpenSSH); the Mac's other services have no Linux authority here.

use super::*;

/// The circled (i) whose tooltip carries the service's live details.
fn details_button(id: &'static str, details: String) -> AnyElement {
    let details = SharedString::from(details);
    div()
        .id(id)
        .size(px(style::INFO_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(details.clone()).build(window, cx)
        })
        .child(glyph("icons/info.svg", style::INFO_BUTTON, label()))
        .into_any_element()
}

/// "active · starts at boot", or why the service cannot be used.
fn service_detail(
    available: bool,
    state: Option<&str>,
    enabled_at_boot: bool,
    missing: &str,
) -> String {
    if available {
        format!(
            "{} · {}",
            state.unwrap_or("unknown"),
            if enabled_at_boot {
                "starts at boot"
            } else {
                "disabled at boot"
            }
        )
    } else {
        missing.to_string()
    }
}

fn service_row(
    icon: &'static str,
    color: Hsla,
    title: &'static str,
    toggle: Toggle,
    details: AnyElement,
) -> AnyElement {
    large_row(tile26(icon, color), title, None)
        .min_h(px(style::SHARING_ROW_HEIGHT))
        .child(toggle)
        .child(details)
        .into_any_element()
}

impl Settings {
    fn sharing_confirmation_card(
        &self,
        id: &'static str,
        service: &'static str,
        enabled: bool,
        cancel: impl Fn(&mut Settings, &mut Context<Settings>) + 'static,
        confirm: impl Fn(&mut Settings, &mut Context<Settings>) + 'static,
        cx: &Context<Self>,
    ) -> Div {
        let cancel_view = cx.entity();
        let confirm_view = cx.entity();
        card(vec![row_base()
            .child(text_block(
                if enabled {
                    format!("Turn on {service}?")
                } else {
                    format!("Turn off {service}?")
                }
                .into(),
                Some("Administrator authorisation may be requested".into()),
            ))
            .child(
                push_button(SharedString::from(format!("cancel-{id}")), "Cancel")
                    .disabled(self.sharing_busy)
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| cancel(settings, cx));
                    }),
            )
            .child(
                Button::new(
                    SharedString::from(format!("confirm-{id}")),
                    if enabled { "Turn On" } else { "Turn Off" },
                )
                .primary()
                .h(px(24.0))
                .busy(self.sharing_busy)
                .disabled(self.sharing_busy)
                .on_click(move |_, _, cx| {
                    confirm_view.update(cx, |settings, cx| confirm(settings, cx));
                }),
            )
            .into_any_element()])
    }

    pub(in crate::controller) fn render_sharing(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = footer_buttons(vec![push_button("refresh-sharing", "Refresh")
            .busy(self.sharing_busy)
            .disabled(self.sharing_loading || self.sharing_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_sharing(cx));
            })
            .into_any_element()]);
        let Some(snapshot) = &self.sharing else {
            return self.pane(vec![
                note_card(if self.sharing_loading {
                    "Reading authoritative sharing capabilities…"
                } else {
                    "Sharing services are unavailable. No local fallback toggles are shown."
                }),
                refresh,
            ]);
        };

        // ---- Content & Media: File Sharing (Samba) ----
        let file = &snapshot.file_sharing;
        let file_toggle_view = view.clone();
        let file_toggle = Toggle::new("file-sharing")
            .checked(file.active && file.enabled_at_boot)
            .disabled(self.sharing_busy || !file.available)
            .on_click(move |enabled, _, cx| {
                file_toggle_view.update(cx, |settings, cx| {
                    settings.file_sharing_confirmation = Some(*enabled);
                    settings.sharing_confirmation = None;
                    cx.notify();
                });
            });
        let mut file_details = vec![
            service_detail(
                file.available,
                file.service_state.as_deref(),
                file.enabled_at_boot,
                "Samba file server is not installed",
            ),
            file.firewall.label("Samba"),
        ];
        if file.shares.is_empty() {
            file_details.push("No shared folders reported".into());
        } else {
            file_details.push(format!(
                "Shared folders: {}",
                file.shares
                    .iter()
                    .map(|share| share.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let mut cards = vec![
            first_section_header("Content & Media"),
            card(vec![service_row(
                "icons/folder.svg",
                hsl(0x30b0c7),
                "File Sharing",
                file_toggle,
                details_button("file-sharing-details", file_details.join("\n")),
            )]),
        ];
        if let Some(enabled) = self.file_sharing_confirmation {
            cards.push(footnote(if enabled {
                "Samba starts now and at every boot. Every configured share becomes reachable under its existing access policy; firewall rules, shares, permissions and credentials are not changed."
            } else {
                "Connected SMB clients may lose access immediately. Samba stops and no longer starts at boot; share definitions are kept."
            }));
            cards.push(self.sharing_confirmation_card(
                "file-sharing",
                "File Sharing",
                enabled,
                |settings, cx| {
                    settings.file_sharing_confirmation = None;
                    cx.notify();
                },
                |settings, cx| settings.confirm_file_sharing(cx),
                cx,
            ));
        }
        if file.shares_truncated {
            cards.push(footnote(
                "The effective Samba share inventory exceeded the bounded display limit.",
            ));
        }
        if let Some(error) = &file.configuration_error {
            cards.push(note_card(error.clone()));
        }
        if file.available && file.active && file.firewall != rmac_sharing::FirewallState::Allows {
            cards.push(footnote(file.firewall_detail.clone().unwrap_or_else(|| {
                "A running SMB service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
            })));
        }

        // ---- Advanced: Remote Login (OpenSSH) ----
        let remote = &snapshot.remote_login;
        let toggle_view = view.clone();
        let remote_toggle = Toggle::new("remote-login")
            .checked(remote.active && remote.enabled_at_boot)
            .disabled(self.sharing_busy || !remote.available)
            .on_click(move |enabled, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.sharing_confirmation = Some(*enabled);
                    settings.file_sharing_confirmation = None;
                    cx.notify();
                });
            });
        let remote_details = [
            service_detail(
                remote.available,
                remote.service_state.as_deref(),
                remote.enabled_at_boot,
                "OpenSSH server is not installed",
            ),
            remote.firewall.label("SSH"),
        ]
        .join("\n");
        cards.push(section_header("Advanced"));
        cards.push(card(vec![service_row(
            "icons/square-terminal.svg",
            hsl(0x8e8e93),
            "Remote Login",
            remote_toggle,
            details_button("remote-login-details", remote_details),
        )]));
        if let Some(enabled) = self.sharing_confirmation {
            cards.push(footnote(if enabled {
                "The system SSH service starts now and at every boot. Firewall rules and authentication policy are not changed."
            } else {
                "Existing SSH sessions may be disconnected and remote access can be lost. The system SSH service stops and no longer starts at boot."
            }));
            cards.push(self.sharing_confirmation_card(
                "remote-login",
                "Remote Login",
                enabled,
                |settings, cx| {
                    settings.sharing_confirmation = None;
                    cx.notify();
                },
                |settings, cx| settings.confirm_remote_login(cx),
                cx,
            ));
        }
        if remote.available
            && remote.active
            && remote.firewall != rmac_sharing::FirewallState::Allows
        {
            cards.push(footnote(remote.firewall_detail.clone().unwrap_or_else(|| {
                "A running SSH service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
            })));
        }
        cards.push(refresh);
        self.pane(cards)
    }
}
