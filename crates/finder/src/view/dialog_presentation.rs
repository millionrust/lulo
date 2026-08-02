use super::*;

impl FinderView {
    pub(super) fn render_recovery(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.recovery_open {
            return None;
        }
        let review = self.recovery_reviews.first()?;
        let presentation = recovery_presentation(&review.action);
        let title = format!(
            "Recover File Operation (1 of {})",
            self.recovery_reviews.len()
        );
        let busy = self.recovery_busy;
        let buttons = vec![
            rmac_ui::dialog_button("recovery-later", "Later", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_recovery(cx)))
                .into_any_element(),
            rmac_ui::dialog_button(
                "recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    pub(super) fn render_conflict(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let batch = self.conflict_batch.as_ref()?;
        let conflict = batch.conflicts.front()?;
        let current = batch
            .conflict_total
            .saturating_sub(batch.conflicts.len())
            .saturating_add(1);
        let title = format!(
            "An Item With This Name Already Exists ({current} of {})",
            batch.conflict_total
        );
        let busy = self.conflict_busy;
        let replace_available =
            conflict.destination_snapshot.is_some() && conflict.source != conflict.destination;
        let buttons = vec![
            rmac_ui::dialog_button("conflict-skip", "Skip", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.resolve_current_conflict(ConflictDecision::Skip, cx)
                }))
                .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-replace",
                "Replace",
                rmac_ui::DialogButtonKind::Destructive,
            )
            .busy(busy)
            .disabled(busy || !replace_available)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::Replace, cx)
            }))
            .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-keep-both",
                if busy { "Checking…" } else { "Keep Both" },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::KeepBoth, cx)
            }))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, conflict_prompt(conflict), buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn render_trash_recovery(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.trash_recovery_open || self.recovery_open {
            return None;
        }
        let review = self.trash_recovery_reviews.first()?;
        let presentation = trash_recovery_presentation(&review.action);
        let title = format!(
            "Recover Trash Operation (1 of {})",
            self.trash_recovery_reviews.len()
        );
        let busy = self.trash_recovery_busy;
        let resolvable = !matches!(
            &review.action,
            trash_store::TrashRecoveryAction::RequiresManualRepair
        );
        let buttons = vec![
            rmac_ui::dialog_button(
                "trash-recovery-later",
                "Later",
                rmac_ui::DialogButtonKind::Normal,
            )
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.close_trash_recovery(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "trash-recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy || !resolvable)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_trash_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn render_delete_confirmation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let confirmation = self.delete_confirmation.as_ref()?;
        let count = confirmation.items.len();
        let name = confirmation
            .items
            .first()
            .and_then(|item| item.original_path.file_name())
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()));
        let title = if count == 1 {
            "Delete Item Permanently?"
        } else {
            "Delete Items Permanently?"
        };
        let buttons = vec![
            rmac_ui::dialog_button(
                "permanent-delete-cancel",
                "Cancel",
                rmac_ui::DialogButtonKind::Normal,
            )
            .on_click(cx.listener(|this, _, _, cx| this.cancel_permanent_delete(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "permanent-delete-confirm",
                "Delete",
                rmac_ui::DialogButtonKind::Destructive,
            )
            .on_click(cx.listener(|this, _, _, cx| this.confirm_permanent_delete(cx)))
            .into_any_element(),
        ];
        Some(
            rmac_ui::alert(
                title,
                permanent_delete_prompt(count, name.as_deref()),
                buttons,
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_open_with(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let picker = self.open_with.as_ref()?;
        let name = picker
            .path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| "this file".into());
        let busy = picker.busy;

        let mut body = div().v_flex().gap_2().px_5().py_4();
        let mut can_open = false;
        let mut selected_is_default = false;
        if let Some(association) = &picker.association {
            body = body.child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child(format!(
                        "Choose an application for “{name}” ({})",
                        association.mime_type
                    )),
            );
            if association.handlers.is_empty() {
                body = body.child(
                    div()
                        .h(px(120.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child("No installed application advertises support for this file type."),
                );
            } else {
                can_open = true;
                let mut rows = Vec::with_capacity(association.handlers.len());
                for (index, application) in association.handlers.iter().enumerate() {
                    let selected = index == picker.selected;
                    let is_default = association.default_application_id.as_deref()
                        == Some(application.id.as_str());
                    if selected {
                        selected_is_default = is_default;
                    }
                    rows.push(
                        div()
                            .id(("open-with-handler", index))
                            .h(px(34.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .when(selected, |element: Stateful<Div>| element.bg(sel()))
                            .when(!selected, |element: Stateful<Div>| {
                                element.hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            })
                            .child(icon(
                                "icons/file-fill.svg",
                                16.0,
                                if selected { white() } else { secondary() },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(if selected { white() } else { label() })
                                    .child(sanitize_dialog_name(&application.name)),
                            )
                            .when(is_default, |element: Stateful<Div>| {
                                element.child(
                                    div()
                                        .text_size(rmac_ui::text_px(11.0))
                                        .text_color(if selected { white() } else { secondary() })
                                        .child("Default"),
                                )
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.choose_open_with(index, cx)),
                            )
                            .into_any_element(),
                    );
                }
                body = body.child(
                    div()
                        .id("open-with-list")
                        .max_h(px(260.0))
                        .overflow_y_scroll()
                        .v_flex()
                        .gap_0p5()
                        .children(rows),
                );

                let checked = selected_is_default || picker.make_default;
                body = body.child(
                    div()
                        .id("open-with-default")
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(if selected_is_default {
                            secondary()
                        } else {
                            label()
                        })
                        .when(!selected_is_default && !busy, |element: Stateful<Div>| {
                            element.cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| this.toggle_open_with_default(cx)),
                            )
                        })
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(if checked {
                                    rmac_ui::mac::accent()
                                } else {
                                    sep()
                                })
                                .bg(if checked {
                                    rmac_ui::mac::accent()
                                } else {
                                    rmac_ui::mac::raised()
                                })
                                .text_color(rmac_ui::mac::on_accent())
                                .child(if checked { "✓" } else { "" }),
                        )
                        .child(if selected_is_default {
                            "This application is already the default"
                        } else {
                            "Always open this file type with this application"
                        }),
                );
            }
        } else {
            body = body.child(
                div()
                    .h(px(150.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary())
                    .child("Finding compatible applications…"),
            );
        }
        if let Some(error) = &picker.error {
            body = body.child(
                div()
                    .id("open-with-error")
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rmac_ui::mac::error_border())
                    .bg(rmac_ui::mac::error_background())
                    .px_3()
                    .py_2()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::danger())
                    .child(error.clone()),
            );
        }

        let buttons = div()
            .h(px(54.0))
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .px_5()
            .border_t_1()
            .border_color(sep())
            .child(
                rmac_ui::dialog_button(
                    "open-with-cancel",
                    if can_open { "Cancel" } else { "Close" },
                    rmac_ui::DialogButtonKind::Normal,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_open_with(cx))),
            )
            .child(
                rmac_ui::dialog_button(
                    "open-with-confirm",
                    if busy { "Opening…" } else { "Open" },
                    rmac_ui::DialogButtonKind::Primary,
                )
                .busy(busy)
                .disabled(busy || !can_open)
                .on_click(cx.listener(|this, _, _, cx| this.confirm_open_with(cx))),
            );

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rmac_ui::mac::scrim())
                .child(
                    div()
                        .w(px(440.0))
                        .max_h(px(520.0))
                        .v_flex()
                        .rounded(px(12.0))
                        .bg(rmac_ui::mac::raised())
                        .border_1()
                        .border_color(sep())
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(48.0))
                                .flex()
                                .items_center()
                                .px_5()
                                .border_b_1()
                                .border_color(sep())
                                .text_size(rmac_ui::text_px(15.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(label())
                                .child("Open With"),
                        )
                        .child(body)
                        .child(buttons),
                )
                .into_any_element(),
        )
    }
}
