//! Software Update settings presentation.

use super::*;

mod dialog;

impl Settings {
    pub(in crate::controller) fn software_update_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let prepare_view = view.clone();
        let refresh = push_button("refresh-update-status", "Check Again")
            .busy(self.updates_busy)
            .disabled(self.updates_loading || self.updates_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_update_status(cx));
            })
            .into_any_element();
        let installed = card(vec![fact_row(
            "Installed",
            self.sysinfo.operating_system.clone(),
        )]);
        let mut body = div().v_flex();

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
                    .child(tile("icons/shield.svg", accent(), style::ROW_ICON))
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
                .child(tile("icons/refresh-cw.svg", accent(), style::ROW_ICON))
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
            body = body.child(card(vec![large_row(
                tile26("icons/shield.svg", hsl(0x34c759)),
                format!("{} packages updated", result.changed_packages),
                Some(subtitle_text(subtitle)),
            )
            .into_any_element()]));
        }

        if self.updates_loading && self.updates.is_none() {
            return body
                .child(
                    Progress::indeterminate()
                        .label("Reading available updates…")
                        .mb_3(),
                )
                .child(installed);
        }

        let Some(snapshot) = &self.updates else {
            return body
                .child(
                    EmptyState::new("Update service unavailable")
                        .message("Install and enable PackageKit, then check again")
                        .error(true),
                )
                .child(installed)
                .child(footer_buttons(vec![refresh]));
        };

        let security = snapshot.security_count();
        let blocked = snapshot.blocked_count();
        let status = if snapshot.updates.is_empty() {
            "Your system is up to date".to_string()
        } else if snapshot.updates.len() == 1 {
            "1 update available".to_string()
        } else {
            format!("{} updates available", snapshot.updates.len())
        };
        let security_line = match security {
            0 => None,
            1 => Some(subtitle_text("1 security update")),
            n => Some(subtitle_text(format!("{n} security updates"))),
        };
        let mut header = large_row(
            tile26("icons/refresh-cw.svg", hsl(0x8e8e93)),
            status,
            security_line,
        );
        if snapshot.can_prepare_install() {
            header = header.child(
                push_button("prepare-update-installation", "Install All…")
                    .disabled(self.updates_busy)
                    .on_click(move |_, _, cx| {
                        prepare_view.update(cx, |settings, cx| settings.prepare_updates(cx));
                    }),
            );
        }
        let mut rows = vec![header.into_any_element()];
        rows.extend(snapshot.updates.iter().map(|update| {
            let detail = if update.summary.is_empty() {
                update.kind.label().to_string()
            } else {
                format!("{} · {}", update.kind.label(), update.summary)
            };
            value_button_row(
                update.name.clone(),
                Some(detail.into()),
                Some(update.version.clone().into()),
                None,
            )
        }));
        body = body.child(card(rows));
        if !snapshot.can_prepare_install() && !snapshot.updates.is_empty() {
            body = body.child(footnote(
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
        if snapshot.truncated {
            body = body.child(footnote(
                "More updates are available than this list can show.",
            ));
        }
        if blocked > 0 {
            body = body.child(footnote("Blocked updates are listed but never installed."));
        }
        body.child(installed).child(footer_buttons(vec![refresh]))
    }
}
