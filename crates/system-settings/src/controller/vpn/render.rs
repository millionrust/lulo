//! VPN pane presentation.

use super::*;

mod dialogs;
mod editor;

impl Settings {
    pub(in crate::controller) fn render_vpn(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let connected = self
            .vpn
            .profiles
            .iter()
            .filter(|profile| profile.state == rmac_network::VpnState::Connected)
            .count();
        let status = match connected {
            0 => "No VPN Connected".to_string(),
            1 => "1 VPN Connected".to_string(),
            count => format!("{count} VPNs Connected"),
        };
        let refresh_view = view.clone();
        let refresh_label = if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            "Deleting…"
        } else if self.vpn_secret_busy {
            "Forgetting…"
        } else if self.vpn_secret_preparing {
            "Preparing…"
        } else if self.vpn_editor_busy {
            "Saving…"
        } else if self.vpn_editor_loading.is_some() {
            "Opening…"
        } else if self.vpn_editor.is_some() {
            "Editing…"
        } else if self.vpn_import_busy {
            "Importing…"
        } else if self.vpn_busy.is_some() {
            "Updating…"
        } else if self.vpn_refreshing {
            "Refreshing…"
        } else {
            "Refresh"
        };
        let refresh_disabled = self.vpn_loading
            || self.vpn_refreshing
            || self.vpn_busy.is_some()
            || self.vpn_import_busy
            || self.vpn_import_preview.is_some()
            || self.vpn_editor_loading.is_some()
            || self.vpn_editor_busy
            || self.vpn_editor.is_some()
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_delete_preparing.is_some()
            || self.vpn_delete_busy
            || self.vpn_delete_preview.is_some();
        let refresh_busy = self.vpn_refreshing
            || self.vpn_busy.is_some()
            || self.vpn_import_busy
            || self.vpn_editor_loading.is_some()
            || self.vpn_editor_busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_delete_preparing.is_some()
            || self.vpn_delete_busy;
        let mut cards = vec![card(vec![value_row(
            "icons/key.svg",
            if connected > 0 {
                hsl(0x34c759)
            } else {
                secondary()
            },
            "Status".into(),
            status.into(),
        )])];
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
                        .child("VPN Configurations"),
                )
                .child(
                    Button::new("vpn-refresh", refresh_label)
                        .ghost()
                        .busy(refresh_busy)
                        .disabled(refresh_disabled)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| settings.refresh_vpn(cx));
                        }),
                ),
        );
        if self.vpn_loading {
            cards.push(note_card("Loading VPN configurations from the system…"));
            return self.pane(cards);
        }
        if !self.vpn.available {
            cards.push(note_card(
                "The system VPN service is not available on this computer.",
            ));
            return self.pane(cards);
        }
        if self.vpn_editor_loading.is_some() {
            cards.push(note_card("Opening current VPN profile details…"));
        }
        if self.vpn_editor.is_some() {
            cards.extend(self.render_vpn_editor(cx));
        }
        if self.vpn.profiles.is_empty() {
            cards.push(note_card(
                "No VPN configurations are installed. Import a configuration below.",
            ));
        } else {
            let rows = self
                .vpn
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| {
                    let id = profile.id.clone();
                    let switch_id = id.clone();
                    let profile_view = view.clone();
                    let delete_id = id.clone();
                    let delete_view = view.clone();
                    let edit_id = id.clone();
                    let edit_view = view.clone();
                    let applying = self.vpn_busy.as_ref() == Some(&id);
                    let connecting = applying && self.vpn_cancellation.is_some();
                    let preparing_delete = self.vpn_delete_preparing.as_ref() == Some(&id);
                    let subtitle = if connecting {
                        format!("{} · Connecting…", profile.service)
                    } else if applying {
                        format!("{} · Disconnecting…", profile.service)
                    } else {
                        format!("{} · {}", profile.service, profile.state.label())
                    };
                    let control = if connecting {
                        Button::new(("vpn-stop", index), "Stop")
                            .on_click(move |_, _, cx| {
                                profile_view
                                    .update(cx, |settings, cx| settings.cancel_vpn_activation(cx));
                            })
                            .into_any_element()
                    } else {
                        Toggle::new(ElementId::from(SharedString::from(format!(
                            "vpn-profile-{index}"
                        ))))
                        .checked(profile.state.is_enabled())
                        .disabled(
                            self.vpn_busy.is_some()
                                || self.vpn_refreshing
                                || self.vpn_import_busy
                                || self.vpn_import_preview.is_some()
                                || self.vpn_editor_loading.is_some()
                                || self.vpn_editor_busy
                                || self.vpn_editor.is_some()
                                || self.vpn_delete_preparing.is_some()
                                || self.vpn_delete_busy
                                || self.vpn_delete_preview.is_some(),
                        )
                        .on_click(move |enabled, _, cx| {
                            let id = switch_id.clone();
                            profile_view.update(cx, |settings, cx| {
                                settings.set_vpn_enabled(id, *enabled, cx)
                            });
                        })
                        .into_any_element()
                    };
                    let controls = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(cfg!(target_os = "linux"), |controls| {
                            controls
                                .child(
                                    Button::new(
                                        ("vpn-edit", index),
                                        if self.vpn_editor_loading.as_ref() == Some(&id) {
                                            "Opening…"
                                        } else {
                                            "Details…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            let id = edit_id.clone();
                                            edit_view.update(cx, |settings, cx| {
                                                settings.start_vpn_edit(id, window, cx)
                                            });
                                        },
                                    ),
                                )
                                .child(
                                    Button::new(
                                        ("vpn-delete", index),
                                        if preparing_delete {
                                            "Preparing…"
                                        } else {
                                            "Delete…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            let id = delete_id.clone();
                                            delete_view.update(cx, |settings, cx| {
                                                settings.request_vpn_delete(id, cx)
                                            });
                                        },
                                    ),
                                )
                        })
                        .child(control);
                    row_base()
                        .child(tile(
                            "icons/key.svg",
                            if profile.state == rmac_network::VpnState::Connected {
                                hsl(0x34c759)
                            } else {
                                accent()
                            },
                            22.0,
                        ))
                        .child(text_block(
                            profile.name.clone().into(),
                            Some(subtitle.into()),
                        ))
                        .child(controls)
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        cards.push(section_header("Import Configuration"));
        if self.vpn_import_loading {
            cards.push(note_card("Checking installed VPN importers…"));
        } else if !self.vpn_import_capabilities.available {
            cards.push(note_card(
                self.vpn_import_capabilities
                    .limitation
                    .clone()
                    .unwrap_or_else(|| "VPN import is unavailable on this system".to_string()),
            ));
        } else if self.vpn_import_capabilities.plugins.is_empty() {
            cards.push(note_card(
                "No reviewed import-capable NetworkManager VPN plugins are installed.",
            ));
        } else {
            let import_rows = self
                .vpn_import_capabilities
                .plugins
                .iter()
                .enumerate()
                .map(|(index, capability)| {
                    let capability_id = capability.id.clone();
                    let import_view = view.clone();
                    row_base()
                        .child(tile("icons/key.svg", accent(), 20.0))
                        .child(text_block(
                            capability.name.clone().into(),
                            Some(capability.format_hint.clone().into()),
                        ))
                        .child(
                            Button::new(("vpn-import", index), "Import…")
                                .disabled(
                                    self.vpn_import_busy
                                        || self.vpn_busy.is_some()
                                        || self.vpn_import_preview.is_some()
                                        || self.vpn_editor_loading.is_some()
                                        || self.vpn_editor_busy
                                        || self.vpn_editor.is_some()
                                        || self.vpn_delete_preparing.is_some()
                                        || self.vpn_delete_busy
                                        || self.vpn_delete_preview.is_some(),
                                )
                                .on_click(move |_, _, cx| {
                                    let capability = capability_id.clone();
                                    import_view.update(cx, |settings, cx| {
                                        settings.choose_vpn_import(capability, cx)
                                    });
                                }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(import_rows));
            if let Some(limitation) = &self.vpn_import_capabilities.limitation {
                cards.push(note_card(limitation.clone()));
            }
        }
        cards.push(note_card(
            "Connections are controlled by the system network service. Authentication prompts are handled by the installed VPN plugin.",
        ));
        self.pane(cards)
    }
}
