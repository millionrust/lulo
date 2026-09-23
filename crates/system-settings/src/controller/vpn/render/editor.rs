//! VPN editor presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_vpn_editor(&self, cx: &Context<Self>) -> Vec<Div> {
        let Some(editor) = &self.vpn_editor else {
            return Vec::new();
        };
        let busy = self.vpn_editor_busy;
        let locked = busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some();
        let view = cx.entity();
        let persistent_view = view.clone();
        let authentication_view = view.clone();
        let authentication_configuration = editor.configuration.clone();
        let cancel_view = view.clone();
        let save_view = view.clone();
        let mut rows = vec![network_field_row(
            "Name",
            "Shown in VPN lists and connection menus",
            &editor.name,
            !locked,
        )];
        if editor.configuration.supports_vpn_options {
            rows.extend([
                network_field_row(
                    "Account Name",
                    "Optional non-secret username; passwords are not read here",
                    &editor.username,
                    !locked,
                ),
                row_base()
                    .child(text_block(
                        "Keep Connection".into(),
                        Some("Ask the VPN plugin to maintain the tunnel when supported".into()),
                    ))
                    .child(
                        Toggle::new("vpn-edit-persistent")
                            .checked(editor.persistent)
                            .disabled(locked)
                            .on_click(move |persistent, _, cx| {
                                persistent_view.update(cx, |settings, cx| {
                                    settings.set_vpn_editor_persistent(*persistent, cx)
                                });
                            }),
                    )
                    .into_any_element(),
                network_field_row(
                    "Connection Timeout",
                    "Seconds; 0 uses the VPN plugin default",
                    &editor.timeout,
                    !locked,
                ),
            ]);
        }
        if editor.configuration.supports_vpn_options {
            rows.push(
                row_base()
                    .child(tile("icons/key.svg", secondary(), style::ROW_ICON))
                    .child(text_block(
                        "Saved Authentication".into(),
                        Some(
                            "Passwords, passphrases, and plugin tokens managed by NetworkManager"
                                .into(),
                        ),
                    ))
                    .child(
                        Button::new(
                            "vpn-forget-authentication",
                            if self.vpn_secret_preparing {
                                "Preparing…"
                            } else {
                                "Forget…"
                            },
                        )
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            let configuration = authentication_configuration.clone();
                            authentication_view.update(cx, |settings, cx| {
                                settings.request_vpn_secret_clear(configuration, cx)
                            });
                        }),
                    )
                    .into_any_element(),
            );
        } else {
            rows.push(value_row(
                "icons/key.svg",
                secondary(),
                "Private Key".into(),
                "Never cleared here".into(),
            ));
        }
        let mut sections = vec![
            section_header(format!("{} · Details", editor.configuration.name)),
            note_card(
                "This editor changes only typed, non-secret fields. NetworkManager and the installed VPN plugin keep the complete plugin configuration, passwords, certificates, and private keys unchanged.",
            ),
            card(rows),
        ];
        if !editor.configuration.supports_vpn_options {
            sections.push(note_card(format!(
                "{} profiles can be renamed here. Their protocol-specific configuration remains under NetworkManager's authority.",
                editor.configuration.service
            )));
        }
        if let Some(error) = &editor.validation_error {
            sections.push(note_card(error.clone()));
        }
        if busy {
            sections.push(
                div()
                    .mb_2()
                    .child(Progress::indeterminate().label("Saving VPN details…")),
            );
        }
        sections.push(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .mb_3()
                .child(
                    Button::new("vpn-edit-cancel", "Cancel")
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_vpn_edit(cx));
                        }),
                )
                .child(
                    Button::new("vpn-edit-save", if busy { "Saving…" } else { "Save" })
                        .primary()
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_vpn_edit(cx));
                        }),
                ),
        );
        sections
    }
}
