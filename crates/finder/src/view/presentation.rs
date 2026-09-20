use super::*;

impl Render for FinderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let native_window_title = rmac_ui::native_window_title(self.title().as_ref(), "Files");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        let layout = responsive_layout::responsive_layout(
            f32::from(window.bounds().size.width),
            self.sidebar_visible,
            self.sidebar_width,
        );
        let info = self.info.clone();
        let multi = self.tabs.len() > 1;
        let menu_at = self.menu_at.clone();
        let menu_purpose = self.menu_purpose;
        let sort_key = self.sort_key;
        let has_sel = self.selection_count() > 0;
        let can_open_with = !self.trash_view
            && !self.applications_view
            && self.selection_count() == 1
            && self.selected_entry().is_some_and(|entry| !entry.is_dir);
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
        let operation_error = operation_error.map(|message| {
            rmac_ui::user_error_message(
                rmac_ui::ErrorSurface::Files,
                message.as_ref(),
                any_recovery_pending,
            )
        });
        let open_with_dialog = self.render_open_with(cx);
        let quick_look_dialog = self.render_quick_look(cx);
        let help_dialog = self.help_open.then(|| {
            rmac_ui::alert(
                "Files Help",
                "Use ⌘1–⌘4 to change views, ⌘↑ for the enclosing folder, Space for Quick Look, and Return to rename the selected item.",
                vec![rmac_ui::dialog_button(
                    "close-files-help",
                    "OK",
                    rmac_ui::DialogButtonKind::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help_open = false;
                    cx.notify();
                }))
                .into_any_element()],
            )
            .into_any_element()
        });
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
            .rounded(px(rmac_ui::mac::radius_large_surface()))
            .overflow_hidden()
            .text_color(label())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.help_open {
                    cx.stop_propagation();
                    if event.keystroke.key.as_str() == "escape" {
                        this.help_open = false;
                        cx.notify();
                    }
                    return;
                }
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
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx) {
                    cx.notify();
                }
            }))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .child(self.render_toolbar(layout, cx))
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
                        .child(div().min_w_0().flex_1().truncate().child(message))
                        .child(
                            Button::new("dismiss-operation-notice", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.operation_notice = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(operation_error, |el, message| {
                el.child(
                    div()
                        .id("operation-error")
                        .relative()
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
                        .child(div().min_w_0().flex_1().pr_20().truncate().child(message))
                        .child(
                            div()
                                .id("resolve-operation-error")
                                .absolute()
                                .right_2()
                                .top(px(5.0))
                                .h(px(24.0))
                                .px_2()
                                .flex()
                                .items_center()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .cursor_pointer()
                                .hover(|button| button.bg(rmac_ui::mac::control_fill_hover()))
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
                        ),
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
                            .child(div().flex_1().child(accessibility::trash_progress_text(
                                operation.as_ref(),
                                processed,
                                total,
                            )))
                            .child(
                                Button::new(
                                    "cancel-trash",
                                    accessibility::cancel_progress_label(cancelling),
                                )
                                .xsmall()
                                .disabled(cancelling)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_trash(cx))),
                            ),
                    )
                },
            )
            .when_some(undo_progress, |el, undo| {
                let status = accessibility::undo_progress_text(&undo);
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
                            Button::new(
                                "cancel-undo",
                                accessibility::cancel_progress_label(undo.cancelling),
                            )
                            .xsmall()
                            .disabled(undo.cancelling)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_undo(cx))),
                        ),
                )
            })
            .when_some(transfer, |el, transfer| {
                let action = accessibility::cancel_progress_label(transfer.cancelling);
                let status = accessibility::transfer_progress_text(&transfer);
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
                            Button::new("cancel-transfer", action)
                                .xsmall()
                                .disabled(transfer.cancelling)
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
                    .when(layout.sidebar_visible, |content| {
                        content.child(self.render_sidebar(cx))
                    })
                    .child(self.render_list(cx)),
            )
            .when_some(info, |el, entry| el.child(self.render_info(&entry, cx)))
            .when_some(menu_at, |el, state| {
                let menu = match menu_purpose {
                    MenuPurpose::Context => Self::build_context_menu(
                        state.position(),
                        has_sel,
                        can_open_with,
                        can_paste,
                        self.trash_view,
                        self.applications_view,
                        undo_label,
                        self.file_words,
                    ),
                    MenuPurpose::Sort => Self::build_sort_menu(state.position(), sort_key),
                };
                el.child(menu.render(&state))
            })
            .when_some(conflict_dialog, |el, dialog| el.child(dialog))
            .when_some(recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(trash_recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
            .when_some(open_with_dialog, |el, dialog| el.child(dialog))
            .when_some(quick_look_dialog, |el, dialog| el.child(dialog))
            .when_some(help_dialog, |el, dialog| el.child(dialog))
    }
}
