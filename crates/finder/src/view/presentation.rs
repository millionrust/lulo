use super::*;

impl FinderView {
    // ---- chrome (toolbar) ----

    pub(super) fn get_info(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error =
                Some("Restore an item before viewing its file information".into());
            cx.notify();
            return;
        }
        self.info = self.selected.iter().next().copied();
        cx.notify();
    }

    /// Recursive platform search of the current folder tree (Return in the search box).
    pub(super) fn recursive_search(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Trash search filters the current list as you type".into());
            cx.notify();
            return;
        }
        let q = self.query.read(cx).value().trim().to_string();
        if q.is_empty() {
            return;
        }
        let cwd = self.cwd.clone();
        let title: SharedString = format!("Search: {}", sanitize_dialog_name(&q)).into();
        let include_hidden = self.show_hidden;
        let (generation, cancel) = self.begin_search();
        self.entries.clear();
        self.selected.clear();
        self.anchor = None;
        self.result_title = Some(title.clone());
        self.search_summary = Some("Searching…".into());
        self.search_relevance_order = true;
        self.view = ViewMode::List;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut options = rmac_search::Options::new(&cancel);
                    options.include_hidden = include_hidden;
                    let mut report = rmac_search::ranked(&cwd, &q, options)?;
                    let reported_matches = report.matches.len();
                    let entries = std::mem::take(&mut report.matches)
                        .into_iter()
                        .filter_map(|search_match| search_entry_for(&cwd, search_match))
                        .collect::<Vec<_>>();
                    report.skipped_errors = report
                        .skipped_errors
                        .saturating_add(reported_matches.saturating_sub(entries.len()));
                    let summary = ranked_search_summary(&report, entries.len());
                    Ok::<_, rmac_search::Error>((entries, summary))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok((entries, summary)) => {
                        this.entries = entries;
                        this.result_title = Some(title);
                        this.search_summary = Some(summary.into());
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => {
                        this.search_summary = None;
                        this.search_relevance_order = false;
                        this.operation_error = Some(ranked_search_error_message(&error).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn tag_click(&mut self, name: SharedString, cx: &mut Context<Self>) {
        self.trash_view = false;
        let title: SharedString = format!("Tag: {name}").into();
        let key = self.sort_key;
        let asc = self.sort_asc;
        let (generation, cancel) = self.begin_search();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut v = rmac_search::tagged(&name, rmac_search::Options::new(&cancel))?
                        .into_iter()
                        .filter_map(|path| entry_for(&path))
                        .collect::<Vec<_>>();
                    sort_entries(&mut v, key, asc);
                    Ok::<_, rmac_search::Error>(v)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.result_title = Some(title);
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show real recently-used files from Spotlight or the XDG bookmark store.
    pub(super) fn recents_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = false;
        let (generation, cancel) = self.begin_search();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut v: Vec<(Entry, std::time::SystemTime)> =
                        rmac_search::recents(rmac_search::Options::new(&cancel))?
                            .into_iter()
                            .filter_map(|path| {
                                let when = std::fs::metadata(&path)
                                    .and_then(|metadata| metadata.modified())
                                    .unwrap_or(std::time::UNIX_EPOCH);
                                entry_for(&path).map(|entry| (entry, when))
                            })
                            .collect();
                    // Most recently modified first, capped so the list stays manageable.
                    v.sort_by(|a, b| b.1.cmp(&a.1));
                    v.truncate(200);
                    Ok::<_, rmac_search::Error>(
                        v.into_iter().map(|(entry, _)| entry).collect::<Vec<_>>(),
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.result_title = Some("Recents".into());
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_info(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.entries.get(ix) else {
            return div();
        };
        let glyph = if e.is_dir {
            "icons/folder-fill.svg"
        } else {
            "icons/file-fill.svg"
        };
        let glyph_color = if e.is_dir { accent() } else { secondary() };

        let mut card = div()
            .w(px(300.0))
            .rounded(px(12.0))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                // header bar with close
                div().h(px(28.0)).flex().items_center().px_2().child(
                    div()
                        .id("info-close")
                        .w(px(14.0))
                        .h(px(14.0))
                        .rounded_full()
                        .bg(hsl(0xff5f57))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.info = None;
                            cx.notify();
                        })),
                ),
            )
            .child(
                // title block
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .pb_3()
                    .px_4()
                    .border_b_1()
                    .border_color(sep())
                    .child(icon(glyph, 56.0, glyph_color))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .text_center()
                            .child(e.name.clone()),
                    ),
            );

        for (k, v) in file_info(e) {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_4()
                    .py_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .child(
                        div()
                            .w(px(96.0))
                            .flex_none()
                            .text_color(secondary())
                            .text_right()
                            .child(format!("{k}:")),
                    )
                    .child(div().flex_1().text_color(label()).child(v)),
            );
        }

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rmac_ui::mac::scrim())
            .child(card.pb_3())
    }
}

impl Render for FinderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = self.info;
        let multi = self.tabs.len() > 1;
        let menu_at = self.menu_at;
        let has_sel = !self.selected.is_empty();
        let can_open_with = !self.trash_view
            && self.selected.len() == 1
            && self
                .selected
                .iter()
                .next()
                .and_then(|index| self.entries.get(*index))
                .is_some_and(|entry| !entry.is_dir);
        let can_paste = !self.clipboard.is_empty();
        let undo_label = self
            .undo_available
            .as_ref()
            .map(|available| available.label.clone());
        let operation_notice = self.operation_notice.clone();
        let operation_error = self.operation_error.clone();
        let transfer = self.transfer.clone();
        let undo_progress = self.undo_operation.clone();
        #[cfg(any(target_os = "linux", test))]
        let trash_progress = self.trash_operation.as_ref().map(|operation| {
            (
                operation.label.clone(),
                operation.processed,
                operation.total,
                operation.cancelling,
            )
        });
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_progress: Option<(SharedString, usize, usize, bool)> = None;
        let recovery_pending = self.pending_operations != 0;
        #[cfg(any(target_os = "linux", test))]
        let trash_recovery_pending = self.trash_pending != 0;
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_recovery_pending = false;
        let any_recovery_pending = recovery_pending || trash_recovery_pending;
        let open_with_dialog = self.render_open_with(cx);
        let quick_look_dialog = self.render_quick_look(cx);
        let conflict_dialog = self.render_conflict(cx);
        let recovery_dialog = self.render_recovery(cx);
        #[cfg(any(target_os = "linux", test))]
        let trash_recovery_dialog = self.render_trash_recovery(cx);
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_recovery_dialog: Option<gpui::AnyElement> = None;
        #[cfg(any(target_os = "linux", test))]
        let delete_dialog = self.render_delete_confirmation(cx);
        #[cfg(not(any(target_os = "linux", test)))]
        let delete_dialog: Option<gpui::AnyElement> = None;
        div()
            .id("files-root")
            .size_full()
            .relative()
            .v_flex()
            .bg(list_bg())
            .text_color(label())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.quick_look.is_some() {
                    cx.stop_propagation();
                    match event.keystroke.key.as_str() {
                        "escape" | "space" => this.close_quick_look(cx),
                        "left" => this.move_quick_look(-1, cx),
                        "right" => this.move_quick_look(1, cx),
                        _ => {}
                    }
                    return;
                }
                if this.open_with.is_some() {
                    cx.stop_propagation();
                    match event.keystroke.key.as_str() {
                        "escape" => this.close_open_with(cx),
                        "up" => this.move_open_with_selection(-1, cx),
                        "down" => this.move_open_with_selection(1, cx),
                        "enter" => this.confirm_open_with(cx),
                        "space" => this.toggle_open_with_default(cx),
                        _ => {}
                    }
                    return;
                }
                #[cfg(any(target_os = "linux", test))]
                if this.delete_confirmation.is_some() {
                    cx.stop_propagation();
                    if event.keystroke.key.as_str() == "escape" {
                        this.cancel_permanent_delete(cx);
                    }
                    return;
                }
                if this.conflict_batch.is_some() {
                    cx.stop_propagation();
                    match conflict_key_intent(event.keystroke.key.as_str(), this.conflict_busy) {
                        Some(ConflictDecision::Skip) => {
                            this.resolve_current_conflict(ConflictDecision::Skip, cx)
                        }
                        Some(ConflictDecision::KeepBoth) => {
                            this.resolve_current_conflict(ConflictDecision::KeepBoth, cx)
                        }
                        _ => {}
                    }
                } else if this.recovery_open {
                    cx.stop_propagation();
                    match recovery_key_intent(event.keystroke.key.as_str(), this.recovery_busy) {
                        Some(RecoveryKeyIntent::Close) => this.close_recovery(cx),
                        Some(RecoveryKeyIntent::Resolve) => this.resolve_current_recovery(cx),
                        None => {}
                    }
                } else {
                    #[cfg(any(target_os = "linux", test))]
                    if this.trash_recovery_open {
                        cx.stop_propagation();
                        match recovery_key_intent(
                            event.keystroke.key.as_str(),
                            this.trash_recovery_busy,
                        ) {
                            Some(RecoveryKeyIntent::Close) => this.close_trash_recovery(cx),
                            Some(RecoveryKeyIntent::Resolve) => {
                                this.resolve_current_trash_recovery(cx)
                            }
                            None => {}
                        }
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .child(self.render_toolbar(cx))
            .when_some(operation_notice, |el, message| {
                el.child(
                    div()
                        .id("operation-notice")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::accent_subtle())
                        .border_b_1()
                        .border_color(rmac_ui::mac::accent_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(label())
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rmac_ui::mac::accent())
                                .text_color(rmac_ui::mac::on_accent())
                                .child("✓"),
                        )
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.operation_notice = None;
                            cx.notify();
                        })),
                )
            })
            .when_some(operation_error, |el, message| {
                el.child(
                    div()
                        .id("operation-error")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::error_background())
                        .border_b_1()
                        .border_color(rmac_ui::mac::error_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rmac_ui::mac::danger())
                                .text_color(rmac_ui::mac::on_danger())
                                .child("!"),
                        )
                        .child(div().flex_1().child(message))
                        .child(if any_recovery_pending {
                            "Review"
                        } else {
                            "Dismiss"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if recovery_pending {
                                this.recovery_open = true;
                            } else {
                                #[cfg(any(target_os = "linux", test))]
                                if trash_recovery_pending {
                                    this.trash_recovery_open = true;
                                    cx.notify();
                                    return;
                                }
                                this.operation_error = None;
                            }
                            cx.notify();
                        })),
                )
            })
            .when_some(
                trash_progress,
                |el, (operation, processed, total, cancelling)| {
                    el.child(
                        div()
                            .h(px(34.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .bg(rmac_ui::mac::accent_subtle())
                            .border_b_1()
                            .border_color(rmac_ui::mac::accent_border())
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(label())
                            .child(
                                div()
                                    .flex_1()
                                    .child(format!("{operation} — {processed} of {total} items")),
                            )
                            .child(
                                div()
                                    .id("cancel-trash")
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(5.0))
                                    .bg(rmac_ui::mac::raised())
                                    .border_1()
                                    .border_color(rmac_ui::mac::accent_border())
                                    .cursor_pointer()
                                    .child(if cancelling {
                                        "Cancelling…"
                                    } else {
                                        "Cancel"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_trash(cx))),
                            ),
                    )
                },
            )
            .when_some(undo_progress, |el, undo| {
                let status = match undo.phase {
                    file_ops::TransferPhase::Scanning => {
                        format!("{} — Checking items", undo.label)
                    }
                    file_ops::TransferPhase::Copying if undo.bytes_processed != 0 => format!(
                        "{} — Restoring {}",
                        undo.label,
                        human_size(undo.bytes_processed)
                    ),
                    file_ops::TransferPhase::Copying => {
                        format!("{} — Restoring item", undo.label)
                    }
                    file_ops::TransferPhase::Finishing => {
                        format!("{} — Finishing safely", undo.label)
                    }
                };
                el.child(
                    div()
                        .id("undo-progress")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::accent_subtle())
                        .border_b_1()
                        .border_color(rmac_ui::mac::accent_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(label())
                        .child(div().flex_1().child(status))
                        .child(
                            div()
                                .id("cancel-undo")
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(rmac_ui::mac::raised())
                                .border_1()
                                .border_color(rmac_ui::mac::accent_border())
                                .cursor_pointer()
                                .child(if undo.cancelling {
                                    "Cancelling…"
                                } else {
                                    "Cancel"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_undo(cx))),
                        ),
                )
            })
            .when_some(transfer, |el, transfer| {
                let action = if transfer.cancelling {
                    "Cancelling…"
                } else {
                    "Cancel"
                };
                let status = match transfer.phase {
                    file_ops::TransferPhase::Scanning => format!(
                        "{} — Scanning {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
                    file_ops::TransferPhase::Copying if transfer.bytes_total > 0 => format!(
                        "{} — {} of {} · {} of {}",
                        transfer.label,
                        transfer.processed,
                        transfer.total,
                        human_size(transfer.bytes_processed),
                        human_size(transfer.bytes_total.max(transfer.bytes_processed))
                    ),
                    file_ops::TransferPhase::Copying => format!(
                        "{} — Copying {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
                    file_ops::TransferPhase::Finishing => format!(
                        "{} — Finishing {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
                };
                el.child(
                    div()
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::accent_subtle())
                        .border_b_1()
                        .border_color(rmac_ui::mac::accent_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(label())
                        .child(div().flex_1().child(status))
                        .child(
                            div()
                                .id("cancel-transfer")
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(rmac_ui::mac::raised())
                                .border_1()
                                .border_color(rmac_ui::mac::accent_border())
                                .cursor_pointer()
                                .child(action)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_transfer(cx))),
                        ),
                )
            })
            .when(multi, |el| el.child(self.render_tabs(cx)))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx)),
            )
            .when_some(info, |el, ix| el.child(self.render_info(ix, cx)))
            .when_some(menu_at, |el, pos| {
                el.child(
                    Self::build_context_menu(
                        pos,
                        has_sel,
                        can_open_with,
                        can_paste,
                        self.trash_view,
                        undo_label,
                    )
                    .render(),
                )
            })
            .when_some(conflict_dialog, |el, dialog| el.child(dialog))
            .when_some(recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(trash_recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
            .when_some(open_with_dialog, |el, dialog| el.child(dialog))
            .when_some(quick_look_dialog, |el, dialog| el.child(dialog))
    }
}
