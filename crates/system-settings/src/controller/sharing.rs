//! Sharing snapshot and reviewed service-mutation lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_sharing_update(
        &mut self,
        result: std::result::Result<rmac_sharing::Snapshot, rmac_sharing::Error>,
    ) {
        self.sharing_loading = false;
        self.sharing_busy = false;
        match result {
            Ok(snapshot) => {
                self.sharing = Some(snapshot);
                self.sharing_error = None;
            }
            Err(error) => {
                self.sharing_error = Some(format!("Could not update Sharing: {error}").into());
            }
        }
    }

    pub(super) fn refresh_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_loading || self.sharing_busy {
            return;
        }
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_sharing_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remote_login(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_remote_login(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_file_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.file_sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_file_sharing(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.file_sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_sharing(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-sharing", "Refresh")
            .busy(self.sharing_busy)
            .disabled(self.sharing_loading || self.sharing_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_sharing(cx));
            });
        let Some(snapshot) = &self.sharing else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/globe.svg", secondary(), 22.0))
                    .child(text_block(
                        "Host sharing services".into(),
                        Some("systemd and firewall authority".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.sharing_loading {
                    "Reading authoritative sharing capabilities…"
                } else {
                    "Sharing services are unavailable. No local fallback toggles are shown."
                }),
            ]);
        };

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
        let mut cards = vec![
            section_header("Remote Login"),
            card(vec![
                row_base()
                    .child(tile("icons/key.svg", accent(), 22.0))
                    .child(text_block(
                        "Remote Login (SSH)".into(),
                        Some(if remote.available {
                            format!(
                                "{} · {}",
                                remote.service_state.as_deref().unwrap_or("unknown"),
                                if remote.enabled_at_boot {
                                    "starts at boot"
                                } else {
                                    "disabled at boot"
                                }
                            )
                            .into()
                        } else {
                            "OpenSSH server is not installed".into()
                        }),
                    ))
                    .child(remote_toggle)
                    .into_any_element(),
                value_row(
                    "icons/shield.svg",
                    if remote.firewall == rmac_sharing::FirewallState::Allows {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    "Firewall".into(),
                    remote.firewall.label("SSH").into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Authoritative state".into(),
                        Some("Live ssh.service · systemd · UFW files".into()),
                    ))
                    .child(refresh)
                    .into_any_element(),
            ]),
        ];

        if let Some(enabled) = self.sharing_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(if enabled {
                "Turn on Remote Login? This enables and starts the system SSH service after administrator authorization. It does not change firewall rules or authentication policy."
            } else {
                "Turn off Remote Login? Existing SSH sessions may be disconnected, and remote access can be lost. This stops and disables the system SSH service after administrator authorization."
            }));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    if enabled {
                        "Confirm enabling Remote Login"
                    } else {
                        "Confirm disabling Remote Login"
                    }
                    .into(),
                    Some("Administrator authorization may be requested".into()),
                ))
                .child(
                    Button::new("cancel-remote-login", "Cancel")
                        .disabled(self.sharing_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.sharing_confirmation = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-remote-login",
                        if enabled { "Turn On" } else { "Turn Off" },
                    )
                    .primary()
                    .busy(self.sharing_busy)
                    .disabled(self.sharing_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_remote_login(cx));
                    }),
                )
                .into_any_element()]));
        }
        if remote.firewall != rmac_sharing::FirewallState::Allows {
            cards.push(note_card(
                remote.firewall_detail.clone().unwrap_or_else(|| {
                    "A running SSH service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
                }),
            ));
        }
        cards.push(section_header("File Sharing"));
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
        cards.push(card(vec![
            row_base()
                .child(tile("icons/hard-drive.svg", accent(), 22.0))
                .child(text_block(
                    "SMB File Sharing".into(),
                    Some(if file.available {
                        format!(
                            "{} · {}",
                            file.service_state.as_deref().unwrap_or("unknown"),
                            if file.enabled_at_boot {
                                "starts at boot"
                            } else {
                                "disabled at boot"
                            }
                        )
                        .into()
                    } else {
                        "Samba file server is not installed".into()
                    }),
                ))
                .child(file_toggle)
                .into_any_element(),
            value_row(
                "icons/shield.svg",
                if file.firewall == rmac_sharing::FirewallState::Allows {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                "Firewall".into(),
                file.firewall.label("Samba").into(),
            ),
            value_row(
                "icons/folder-symlink.svg",
                secondary(),
                "Effective shares".into(),
                if file.shares.is_empty() {
                    "None reported".into()
                } else {
                    format!("{} configured", file.shares.len()).into()
                },
            ),
        ]));
        if !file.shares.is_empty() {
            cards.push(card(
                file.shares
                    .iter()
                    .map(|share| {
                        value_row(
                            "icons/folder-symlink.svg",
                            secondary(),
                            share.name.clone().into(),
                            "SMB share".into(),
                        )
                    })
                    .collect(),
            ));
        }
        if let Some(enabled) = self.file_sharing_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(if enabled {
                "Turn on File Sharing? This enables and starts Samba after administrator authorization. Every effective configured share may become reachable under its existing access policy. Firewall rules, share definitions, file permissions, and credentials are not changed."
            } else {
                "Turn off File Sharing? Connected SMB clients may lose access immediately. This stops and disables Samba after administrator authorization without deleting share definitions."
            }));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    if enabled {
                        "Confirm enabling File Sharing"
                    } else {
                        "Confirm disabling File Sharing"
                    }
                    .into(),
                    Some("Administrator authorization may be requested".into()),
                ))
                .child(
                    Button::new("cancel-file-sharing", "Cancel")
                        .disabled(self.sharing_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.file_sharing_confirmation = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-file-sharing",
                        if enabled { "Turn On" } else { "Turn Off" },
                    )
                    .primary()
                    .busy(self.sharing_busy)
                    .disabled(self.sharing_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_file_sharing(cx));
                    }),
                )
                .into_any_element()]));
        }
        if file.shares_truncated {
            cards.push(note_card(
                "The effective Samba share inventory exceeded the bounded display limit.",
            ));
        }
        if let Some(error) = &file.configuration_error {
            cards.push(note_card(error.clone()));
        }
        if file.firewall != rmac_sharing::FirewallState::Allows {
            cards.push(note_card(file.firewall_detail.clone().unwrap_or_else(|| {
                "A running SMB service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
            })));
        }
        cards.push(note_card(
            "The switch controls only smbd.service runtime and boot state. Share definitions, permissions, credentials, and firewall policy remain separate authorities. rmac does not present AirDrop because Linux has no compatible local authority.",
        ));
        self.pane(cards)
    }
}
