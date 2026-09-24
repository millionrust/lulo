//! Software Update installation review dialog.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_update_install_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let plan = self.updates_plan.as_ref()?;
        let requested = plan.requested.len();
        let changes = plan.changes.len();
        let installs = plan.change_count(rmac_updates::ChangeKind::Install);
        let removals = plan.change_count(rmac_updates::ChangeKind::Remove)
            + plan.change_count(rmac_updates::ChangeKind::Obsolete);
        let downgrades = plan.change_count(rmac_updates::ChangeKind::Downgrade);
        let destructive_preview = plan
            .changes
            .iter()
            .filter(|change| change.kind.is_destructive())
            .take(8)
            .map(|change| {
                format!(
                    "{} {} ({})",
                    change.name,
                    change.version,
                    change.kind.label()
                )
            })
            .collect::<Vec<_>>();
        let hidden_destructive = removals + downgrades - destructive_preview.len();
        let summary = format!(
            "PackageKit will apply {requested} requested updates through {changes} verified package changes. Dependencies are included in this preview."
        );
        let mut rows = vec![
            value_row(
                "icons/refresh-cw.svg",
                accent(),
                "Requested updates".into(),
                requested.to_string().into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Additional installs".into(),
                installs.to_string().into(),
            ),
        ];
        if removals > 0 {
            rows.push(value_row(
                "icons/shield.svg",
                hsl(0xff3b30),
                "Removals or replacements".into(),
                removals.to_string().into(),
            ));
        }
        if downgrades > 0 {
            rows.push(value_row(
                "icons/info.svg",
                hsl(0xff9500),
                "Downgrades".into(),
                downgrades.to_string().into(),
            ));
        }
        let view = cx.entity();
        let cancel_view = view.clone();
        let install_view = view.clone();
        let content = div()
            .w(px(460.0))
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
                            .child("Install system updates?"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(summary),
                    ),
            )
            .child(card(rows))
            .when(plan.has_destructive_changes(), |dialog| {
                let mut warning = format!(
                    "This verified plan changes packages destructively: {}.",
                    destructive_preview.join(", ")
                );
                if hidden_destructive > 0 {
                    warning.push_str(&format!(" Plus {hidden_destructive} more shown in the counts above."));
                }
                dialog.child(note_card(warning))
            })
            .when_some(plan.restart.label(), |dialog, restart| {
                dialog.child(note_card(format!("Expected after installation: {restart}.")))
            })
            .child(note_card(
                "Lulo OS installs only the exact revalidated plan and keeps PackageKit's trusted-only flag enabled. Authorization may be requested.",
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_update_plan(cx));
                        }),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-confirm",
                            "Install Updates",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(move |_, _, cx| {
                            install_view
                                .update(cx, |settings, cx| settings.confirm_update_plan(cx));
                        }),
                    ),
            );
        Some(
            rmac_ui::dialog("update-install-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_update_plan(cx);
                        }
                        "enter" => {
                            cx.stop_propagation();
                            this.confirm_update_plan(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}
