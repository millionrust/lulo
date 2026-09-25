//! Software Update sheets: the More Info (ⓘ) sheet, the Automatic Updates
//! sheet (both measured from macOS 26.2, design-lab/software-update.html),
//! and the confirmation a plan with removals or downgrades still needs.

use super::*;

/// More Info: 589 × 588, a table of 27 pt rows (four visible), then the
/// selected item's details.
const INFO_SHEET_WIDTH: f32 = 589.0;
const INFO_SHEET_HEIGHT: f32 = 588.0;
const INFO_ROW_HEIGHT: f32 = 27.0;
const INFO_VISIBLE_ROWS: usize = 4;
const INFO_TITLE_GAP: f32 = 9.0;
const INFO_DETAIL_TOP: f32 = 11.0;
const INFO_DETAIL_HEIGHT: f32 = 306.0;
/// Version column ends 107 before the table's right edge; sizes end 10
/// before it.
const INFO_VERSION_RIGHT: f32 = 107.0;
const INFO_SIZE_WIDTH: f32 = 90.0;
/// Automatic Updates: 470 × 274.
const AUTO_SHEET_WIDTH: f32 = 470.0;
const AUTO_SHEET_HEIGHT: f32 = 274.0;

struct InfoRow {
    key: String,
    name: String,
    version: String,
    size: Option<u64>,
}

fn info_rows(snapshot: &rmac_updates::Snapshot) -> Vec<InfoRow> {
    let catalog = rmac_updates::Catalog::from_snapshot(snapshot);
    let mut rows = Vec::new();
    if let Some(lulo) = &catalog.lulo_os {
        rows.push(InfoRow {
            key: rmac_updates::LULO_OS_ITEM.to_owned(),
            name: lulo.title(),
            version: lulo.version.clone().unwrap_or_default(),
            size: lulo.download_size,
        });
    }
    rows.extend(catalog.other.iter().map(|update| InfoRow {
        key: update.package_id.clone(),
        name: update.name.clone(),
        version: update.version.clone(),
        size: snapshot.download_sizes.get(&update.package_id).copied(),
    }));
    rows
}

fn sheet_title(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(style::ROW_PADDING))
        .text_size(rmac_ui::text_px(13.0))
        .line_height(px(16.0))
        .font_weight(rmac_ui::mac::BOLD)
        .text_color(label())
        .child(text.into())
}

impl Settings {
    pub(in crate::controller) fn render_update_info_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let sheet = self.updates_info_sheet.as_ref()?;
        let snapshot = self.updates.as_ref()?;
        let rows = info_rows(snapshot);
        let view = cx.entity();
        let catalog = rmac_updates::Catalog::from_snapshot(snapshot);

        let table_rows = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let selected = row.key == sheet.selected;
                let ticked = !sheet.unticked.contains(&row.key);
                let text = if selected { gpui::white() } else { label() };
                let fill = if selected {
                    style::sidebar_selection_focused()
                } else if index % 2 == 0 {
                    style::info_table_stripe()
                } else {
                    card_bg()
                };
                let select_view = view.clone();
                let select_key = row.key.clone();
                let toggle_view = view.clone();
                let toggle_key = row.key.clone();
                div()
                    .id(SharedString::from(format!("software-update-row-{index}")))
                    .flex()
                    .items_center()
                    .h(px(INFO_ROW_HEIGHT))
                    .flex_none()
                    .pl(px(9.0))
                    .pr(px(style::ROW_PADDING))
                    .gap(px(21.0))
                    .bg(fill)
                    .role(gpui::Role::Row)
                    .aria_label(row.name.clone())
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let key = select_key.clone();
                        select_view
                            .update(cx, |settings, cx| settings.select_update_info_row(key, cx));
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("software-update-tick-{index}")))
                            .role(gpui::Role::CheckBox)
                            .aria_label(format!("Include {}", row.name))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                let key = toggle_key.clone();
                                toggle_view.update(cx, |settings, cx| {
                                    settings.toggle_update_info_row(key, cx)
                                });
                            })
                            .child(form_checkbox(ticked)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(text)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(text)
                            .mr(px(INFO_VERSION_RIGHT
                                - INFO_SIZE_WIDTH
                                - style::ROW_PADDING))
                            .child(row.version.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(INFO_SIZE_WIDTH))
                            .flex()
                            .justify_end()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(text)
                            .child(row.size.map(rmac_updates::format_size).unwrap_or_default()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let visible = rows.len().clamp(1, INFO_VISIBLE_ROWS) as f32;
        let table = div()
            .id("software-update-table")
            .v_flex()
            .h(px(visible * INFO_ROW_HEIGHT))
            .rounded(px(style::GROUP_RADIUS))
            .overflow_y_scroll()
            .bg(card_bg())
            .role(gpui::Role::Table)
            .aria_label("Available updates")
            .children(table_rows);

        let detail = match rows.iter().find(|row| row.key == sheet.selected) {
            Some(row) if row.key == rmac_updates::LULO_OS_ITEM => {
                let notes = snapshot.release_notes.clone().unwrap_or_else(|| {
                    catalog
                        .lulo_os
                        .as_ref()
                        .map(|lulo| package_list_notes(&lulo.packages))
                        .unwrap_or_default()
                });
                Some((row.name.clone(), notes))
            }
            Some(row) => catalog
                .other
                .iter()
                .find(|update| update.package_id == row.key)
                .map(|update| {
                    let text = if update.summary.is_empty() {
                        update.kind.label().to_string()
                    } else {
                        format!("{} · {}", update.kind.label(), update.summary)
                    };
                    (
                        format!("{} {}", update.name, row.version),
                        rmac_updates::ReleaseNotes {
                            blocks: vec![rmac_updates::NotesBlock::Paragraph(text)],
                        },
                    )
                }),
            None => None,
        };
        let detail_card = div()
            .id("software-update-detail")
            .mt(px(INFO_DETAIL_TOP))
            .h(px(INFO_DETAIL_HEIGHT))
            .rounded(px(style::GROUP_RADIUS))
            .bg(card_bg())
            .overflow_y_scroll()
            .when_some(detail, |card, (title, notes)| {
                card.child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.0))
                        .px(px(style::ROW_PADDING))
                        .pt(px(style::ROW_PADDING))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(15.0))
                                .font_weight(rmac_ui::mac::BOLD)
                                .text_color(label())
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(15.0))
                                .text_color(secondary())
                                .child("— Restart Required"),
                        ),
                )
                .child(release_notes(&notes.blocks))
                .child(row_separator())
                .child(card_footnote(
                    "Once downloaded, updates are installed when you restart.",
                ))
            });

        let body = div()
            .v_flex()
            .child(sheet_title("Updates are available for your computer").mb(px(INFO_TITLE_GAP)))
            .child(table)
            .child(detail_card)
            .into_any_element();

        let ticked = rows
            .iter()
            .filter(|row| !sheet.unticked.contains(&row.key))
            .count();
        let cancel_view = view.clone();
        let update_view = view.clone();
        let footer = vec![
            rmac_ui::dialog_button(
                "software-update-info-cancel",
                "Cancel",
                rmac_ui::DialogButtonKind::Normal,
            )
            .on_click(move |_, _, cx| {
                cancel_view.update(cx, |settings, cx| settings.close_update_info_sheet(cx));
            })
            .into_any_element(),
            div().flex_1().into_any_element(),
            rmac_ui::dialog_button(
                "software-update-info-update",
                "Update Now",
                rmac_ui::DialogButtonKind::Primary,
            )
            .disabled(ticked == 0 || self.updates_busy || !snapshot.install_supported)
            .on_click(move |_, _, cx| {
                update_view.update(cx, |settings, cx| settings.update_info_selection(cx));
            })
            .into_any_element(),
        ];
        Some(
            settings_sheet(
                "software-update-info",
                INFO_SHEET_WIDTH,
                INFO_SHEET_HEIGHT,
                None,
                body,
                footer,
            )
            .into_any_element(),
        )
    }

    pub(in crate::controller) fn render_update_auto_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.updates_auto_sheet {
            return None;
        }
        let auto = self.updates_auto;
        let view = cx.entity();
        let download_view = view.clone();
        let lulo_view = view.clone();
        let security_view = view.clone();
        let done_view = view.clone();
        let body = div()
            .v_flex()
            .child(sheet_title("Automatically").mb(px(style::ROW_PADDING)))
            .child(card(vec![
                switch_row(
                    "software-update-auto-download",
                    "Download new updates when available",
                    None,
                    auto.download,
                    true,
                    move |value, _, cx| {
                        download_view.update(cx, |settings, cx| {
                            settings.set_automatic_updates(|auto| auto.download = value, cx)
                        });
                    },
                ),
                switch_row(
                    "software-update-auto-lulo",
                    "Install Lulo OS updates",
                    None,
                    auto.download && auto.install_lulo_os,
                    auto.download,
                    move |value, _, cx| {
                        lulo_view.update(cx, |settings, cx| {
                            settings.set_automatic_updates(|auto| auto.install_lulo_os = value, cx)
                        });
                    },
                ),
            ]))
            .child(card(vec![switch_row(
                "software-update-auto-security",
                "Install system data files and security updates",
                None,
                auto.download && auto.install_security,
                auto.download,
                move |value, _, cx| {
                    security_view.update(cx, |settings, cx| {
                        settings.set_automatic_updates(|auto| auto.install_security = value, cx)
                    });
                },
            )]))
            .when_some(self.updates_auto_error.clone(), |body, error| {
                body.child(footnote(error))
            })
            .into_any_element();
        let footer = vec![sheet_default_button(
            "software-update-auto-done",
            "Done",
            move |_, cx| {
                done_view.update(cx, |settings, cx| settings.close_update_auto_sheet(cx));
            },
        )];
        Some(
            settings_sheet(
                "software-update-automatic",
                AUTO_SHEET_WIDTH,
                AUTO_SHEET_HEIGHT,
                None,
                body,
                footer,
            )
            .into_any_element(),
        )
    }

    /// A plan that removes, replaces or downgrades packages is never
    /// downloaded without this confirmation.
    pub(in crate::controller) fn render_update_install_dialog(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let plan = self.updates_plan.as_ref()?;
        let requested = plan.requested.len();
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
        let hidden_destructive = (removals + downgrades).saturating_sub(destructive_preview.len());
        let mut changes = format!(
            "This update changes other software: {}.",
            destructive_preview.join(", ")
        );
        if hidden_destructive > 0 {
            changes.push_str(&format!(" And {hidden_destructive} more."));
        }
        let summary = format!(
            "{requested} updates, {installs} new packages, {removals} removed or replaced, {downgrades} downgraded. Nothing changes until you restart."
        );
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
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::BOLD)
                            .text_color(label())
                            .child("This update removes or replaces software"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(secondary())
                            .child(summary),
                    ),
            )
            .child(note_card(changes))
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
                            cancel_view.update(cx, |settings, cx| settings.cancel_update_plan(cx));
                        }),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-confirm",
                            "Download",
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
