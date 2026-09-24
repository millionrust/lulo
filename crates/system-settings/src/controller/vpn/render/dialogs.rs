//! VPN import, saved-secret, and deletion dialogs.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_vpn_secret_clear_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let preview = self.vpn_secret_preview.as_ref()?;
        let busy = self.vpn_secret_busy;
        let consequence = if preview.currently_connected {
            "The current connection will stay active. NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens, then the installed VPN plugin may ask for them after the next disconnect. This cannot be undone by Lulo OS."
        } else {
            "NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens. The installed VPN plugin may ask for them on the next connection. This cannot be undone by Lulo OS."
        };
        let content = div()
            .w(px(440.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Forget authentication for “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .child(note_card(
                "The VPN profile, server configuration, certificates, and current tunnel are not deleted. Native WireGuard private keys are never handled by this action.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Forgetting saved authentication…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_vpn_secret_clear(cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-confirm",
                            "Forget",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_vpn_secret_clear(cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-secret-clear-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_secret_clear(cx);
                        }
                        "enter" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_secret_clear(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(in crate::controller) fn render_vpn_import_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let preview = self.vpn_import_preview.as_ref()?;
        let busy = self.vpn_import_busy;
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Import “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(format!(
                                "{} recognized “{}”.",
                                preview.service, preview.source_name
                            )),
                    ),
            )
            .child(note_card(
                "The profile is temporary and cannot connect automatically. Import saves it to NetworkManager; passwords and keys remain under NetworkManager and the VPN plugin’s authority.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Finishing VPN import…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(false, cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-confirm",
                            "Import",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(true, cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-import-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(false, cx);
                        }
                        "enter" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(true, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(in crate::controller) fn render_vpn_delete_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let preview = self.vpn_delete_preview.as_ref()?;
        let busy = self.vpn_delete_busy;
        let consequence = if preview.will_disconnect {
            "The active connection will disconnect first. The saved profile and its NetworkManager-managed secrets will then be removed. This cannot be undone."
        } else {
            "The saved profile and its NetworkManager-managed secrets will be removed. This cannot be undone."
        };
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_card()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Delete “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Deleting VPN profile…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_vpn_delete(cx))),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-confirm",
                            "Delete",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_vpn_delete(cx))),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-delete-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_delete(cx);
                        }
                        "enter" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_delete(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}
