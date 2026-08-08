//! Software Update settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn software_update_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let prepare_view = view.clone();
        let refresh = Button::new("refresh-update-status", "Check Again")
            .busy(self.updates_busy)
            .disabled(self.updates_loading || self.updates_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_update_status(cx));
            });
        let mut body = div().v_flex().child(card(vec![
            value_row(
                "icons/info.svg",
                secondary(),
                "Current version".into(),
                self.sysinfo.operating_system.clone().into(),
            ),
            row_base()
                .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                .child(text_block(
                    "Package updates".into(),
                    Some("PackageKit · configured repositories".into()),
                ))
                .child(refresh)
                .into_any_element(),
        ]));

        if self.updates_preparing {
            let cancel_view = view.clone();
            let cancel_requested = self
                .updates_cancellation
                .as_ref()
                .is_some_and(rmac_updates::Cancellation::is_cancelled);
            return body
                .child(
                    Progress::indeterminate()
                        .label("Refreshing and simulating the trusted package plan…")
                        .mb_3(),
                )
                .child(card(vec![row_base()
                    .child(tile("icons/shield.svg", accent(), 22.0))
                    .child(text_block(
                        "Verifying dependencies".into(),
                        Some("No package changes have started".into()),
                    ))
                    .child(
                        Button::new(
                            "cancel-update-preparation",
                            if cancel_requested {
                                "Cancelling…"
                            } else {
                                "Cancel"
                            },
                        )
                        .busy(cancel_requested)
                        .disabled(cancel_requested)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_update_operation(cx));
                        }),
                    )
                    .into_any_element()]));
        }

        if self.updates_installing {
            let progress = self.updates_progress.clone().unwrap_or_default();
            let mut progress_label = progress.phase.label().to_string();
            if let Some(package) = &progress.current_package {
                progress_label.push_str(&format!(" · {package}"));
            }
            if let Some(remaining) = progress.remaining_seconds {
                progress_label.push_str(&format!(
                    " · about {}",
                    format_power_duration(u64::from(remaining))
                ));
            }
            let progress_element = match progress.percentage {
                Some(percentage) => Progress::new(f32::from(percentage) / 100.0),
                None => Progress::indeterminate(),
            }
            .label(progress_label)
            .mb_3();
            let cancel_requested = self
                .updates_cancellation
                .as_ref()
                .is_some_and(rmac_updates::Cancellation::is_cancelled);
            let cancel_view = view.clone();
            return body.child(progress_element).child(card(vec![row_base()
                .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                .child(text_block(
                    "Installing trusted updates".into(),
                    Some("PackageKit owns the transaction; do not turn off this computer".into()),
                ))
                .child(
                    Button::new(
                        "cancel-update-installation",
                        if cancel_requested {
                            "Cancelling…"
                        } else {
                            "Cancel"
                        },
                    )
                    .busy(cancel_requested)
                    .disabled(!progress.allow_cancel || cancel_requested)
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| settings.cancel_update_operation(cx));
                    }),
                )
                .into_any_element()]));
        }

        if let Some(result) = &self.updates_result {
            let subtitle = result
                .restart
                .label()
                .map(str::to_owned)
                .unwrap_or_else(|| "No restart was requested by PackageKit".into());
            body = body.child(card(vec![value_row(
                "icons/shield.svg",
                hsl(0x34c759),
                format!("{} packages updated", result.changed_packages).into(),
                subtitle.into(),
            )]));
        }

        if self.updates_loading && self.updates.is_none() {
            return body.child(
                Progress::indeterminate()
                    .label("Reading available updates…")
                    .mb_3(),
            );
        }

        let Some(snapshot) = &self.updates else {
            return body
                .child(
                    EmptyState::new("Update service unavailable")
                        .message("Install and enable PackageKit, then check again")
                        .error(true),
                )
                .child(note_card(
                    "No package state is guessed from local files or command output.",
                ));
        };

        let security = snapshot.security_count();
        let blocked = snapshot.blocked_count();
        let status = if snapshot.updates.is_empty() {
            "Your system is up to date".to_string()
        } else if security > 0 {
            format!(
                "{} updates available · {security} security",
                snapshot.updates.len()
            )
        } else {
            format!("{} updates available", snapshot.updates.len())
        };
        body = body.child(card(vec![value_row(
            "icons/shield.svg",
            if security > 0 {
                hsl(0xff3b30)
            } else {
                hsl(0x34c759)
            },
            "Status".into(),
            status.into(),
        )]));

        if snapshot.can_prepare_install() {
            body = body.child(card(vec![row_base()
                .child(tile("icons/shield.svg", accent(), 22.0))
                .child(text_block(
                    "Install all trusted updates".into(),
                    Some("Refresh, simulate dependencies, then confirm the exact plan".into()),
                ))
                .child(
                    Button::new("prepare-update-installation", "Install All…")
                        .primary()
                        .disabled(self.updates_busy)
                        .on_click(move |_, _, cx| {
                            prepare_view.update(cx, |settings, cx| settings.prepare_updates(cx));
                        }),
                )
                .into_any_element()]));
        } else if !snapshot.updates.is_empty() {
            body = body.child(note_card(
                snapshot
                    .install_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| {
                        if snapshot.truncated {
                            "The complete update set is too large to confirm safely.".into()
                        } else if snapshot.installable_count() == 0 {
                            "Every reported update is currently blocked by PackageKit.".into()
                        } else {
                            "The PackageKit backend cannot install updates on this system.".into()
                        }
                    }),
            ));
        }

        if !snapshot.updates.is_empty() {
            body = body.child(section_header("Available Updates"));
            let rows = snapshot
                .updates
                .iter()
                .map(|update| {
                    let detail = if update.summary.is_empty() {
                        update.kind.label().to_string()
                    } else {
                        format!("{} · {}", update.kind.label(), update.summary)
                    };
                    row_base()
                        .child(tile(
                            "icons/refresh-cw.svg",
                            match update.kind {
                                rmac_updates::UpdateKind::Security => hsl(0xff3b30),
                                rmac_updates::UpdateKind::Blocked => hsl(0xff9500),
                                _ => accent(),
                            },
                            22.0,
                        ))
                        .child(text_block(update.name.clone().into(), Some(detail.into())))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(secondary())
                                .child(update.version.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            body = body.child(card(rows));
        }
        if snapshot.truncated {
            body = body.child(note_card(
                "More updates are available than this bounded view can display.",
            ));
        }
        if blocked > 0 {
            body = body.child(note_card(
                "Blocked updates are shown for awareness but are never included in rmac's installation plan.",
            ));
        }
        body.child(note_card(
            "PackageKit refreshes, simulates, downloads, and installs without shell commands. rmac never retries with untrusted packages and always rereads remaining updates afterward.",
        ))
    }
}
