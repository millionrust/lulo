use super::*;

impl Render for FinderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The application menu is shared by every Files window, so only the
        // active window may publish its window-specific validation state.
        if window.is_window_active() {
            self.publish_app_menu_state(cx);
        }
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
        let window_active = window.is_window_active();
        let window_height = f32::from(window.bounds().size.height);
        let multi = self.tabs.len() > 1;
        let menu_at = self.menu_at.clone();
        let menu_purpose = self.menu_purpose;
        let sort_key = self.sort_key;
        let compress_label = self.compress_menu_label();
        let can_open_with = !self.trash_view
            && !self.applications_view
            && self.selection_count() == 1
            && self.selected_entry().is_some_and(|entry| !entry.is_dir);
        let can_paste = self.can_paste();
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
        let go_to_sheet = self.render_go_to_folder(cx);
        let archive_sheet = self.render_archive_job(cx);
        let archive_alert = self.render_archive_alert(cx);
        let rename_alert = self.rename_conflict.clone().map(|title| {
            rmac_ui::alert(
                title,
                "",
                vec![rmac_ui::dialog_button(
                    "rename-conflict-ok",
                    "OK",
                    rmac_ui::DialogButtonKind::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.rename_conflict = None;
                    cx.notify();
                }))
                .into_any_element()],
            )
            .into_any_element()
        });
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
        let missing_favourite = self.missing_favourite.as_ref().map(|path| {
            let name = rmac_finder::sidebar_favourites::label(path);
            rmac_ui::alert(
                "The item can't be found",
                format!("“{name}” may have been moved or deleted. Remove it from the Sidebar?"),
                vec![
                    rmac_ui::dialog_button(
                        "missing-favourite-cancel",
                        "Keep",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.missing_favourite = None;
                        cx.notify();
                    }))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "missing-favourite-remove",
                        "Remove",
                        rmac_ui::DialogButtonKind::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(path) = this.missing_favourite.take() {
                            this.remove_sidebar_favourite(&path, cx);
                        }
                    }))
                    .into_any_element(),
                ],
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
            .drag_over::<DraggedSidebarItem>(|style, _, _, _| style)
            .on_drop(cx.listener(|this, item: &DraggedSidebarItem, _, cx| {
                this.remove_sidebar_item(&item.0, cx);
            }))
            .size_full()
            .relative()
            .flex()
            .bg(list_bg())
            .rounded(px(rmac_ui::mac::radius_large_surface()))
            .overflow_hidden()
            .text_color(label())
            .on_action(cx.listener(|this, _: &ShowViewOptions, _, cx| this.toggle_view_options(cx)))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.view_options_open && event.keystroke.key.as_str() == "escape" {
                    this.close_view_options(cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.go_to.is_some() {
                    // Typing goes to the path field; these keys drive the sheet.
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.close_go_to_folder(window, cx);
                        }
                        "up" => {
                            cx.stop_propagation();
                            this.move_go_to_highlight(-1, cx);
                        }
                        "down" => {
                            cx.stop_propagation();
                            this.move_go_to_highlight(1, cx);
                        }
                        _ => {}
                    }
                    return;
                }
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
                        "left" | "up" => this.move_quick_look(-1, cx),
                        "right" | "down" => this.move_quick_look(1, cx),
                        _ => {}
                    }
                    return;
                }
                if this.open_with.is_some() {
                    let browsing = this
                        .open_with
                        .as_ref()
                        .is_some_and(|picker| picker.browse.is_some());
                    cx.stop_propagation();
                    match event.keystroke.key.as_str() {
                        "escape" if browsing => this.cancel_choose_application(cx),
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
                    return;
                } else if this.recovery_open {
                    cx.stop_propagation();
                    match recovery_key_intent(event.keystroke.key.as_str(), this.recovery_busy) {
                        Some(RecoveryKeyIntent::Close) => this.close_recovery(cx),
                        Some(RecoveryKeyIntent::Resolve) => this.resolve_current_recovery(cx),
                        None => {}
                    }
                    return;
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
                        return;
                    }
                }
                if event.keystroke.key.as_str() == "escape"
                    && (this.search_open
                        || !this.query.read(cx).value().is_empty()
                        || this.showing_recursive_search())
                {
                    cx.stop_propagation();
                    let recursive_search_active = this.showing_recursive_search();
                    this.search_open = false;
                    if recursive_search_active {
                        // Clear the result mode before changing the input so
                        // the Change subscription does not reload twice.
                        this.reload(cx);
                    }
                    if !this.query.read(cx).value().is_empty() {
                        this.query
                            .update(cx, |state, cx| state.set_value("", window, cx));
                    } else if !recursive_search_active {
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx) {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.close_finder_window(window, cx);
            }))
            .when(layout.sidebar_visible, |root| {
                root.child(self.render_sidebar(cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .v_flex()
                    .child(self.render_toolbar(layout, cx))
                    .when_some(operation_notice, |el, message| {
                        let checking = message.as_ref() == DIRECTORY_STALL_NOTICE;
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
                                .when(!checking, |el| {
                                    el.child(
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
                                })
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
                                        .hover(|button| {
                                            button.bg(rmac_ui::mac::control_fill_hover())
                                        })
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
                                    .child(div().flex_1().child(
                                        accessibility::trash_progress_text(
                                            operation.as_ref(),
                                            processed,
                                            total,
                                        ),
                                    ))
                                    .child(
                                        Button::new(
                                            "cancel-trash",
                                            accessibility::cancel_progress_label(cancelling),
                                        )
                                        .xsmall()
                                        .disabled(cancelling)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.cancel_trash(cx)),
                                        ),
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
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.cancel_transfer(cx)),
                                        ),
                                ),
                        )
                    })
                    .when(multi || self.show_tab_bar, |el| {
                        el.child(self.render_tabs(cx))
                    })
                    .when(self.trash_view, |el| el.child(self.render_trash_bar(cx)))
                    .child(self.render_list(
                        window_active,
                        window_height,
                        f32::from(window.bounds().size.width)
                            - if layout.sidebar_visible {
                                self.sidebar_width
                            } else {
                                0.0
                            },
                        cx,
                    )),
            )
            .when_some(go_to_sheet, |el, sheet| el.child(sheet))
            .when_some(menu_at, |el, state| {
                let menu = match menu_purpose {
                    MenuPurpose::Context => Self::build_context_menu(
                        state.position(),
                        sort_key,
                        compress_label,
                        SelectionMenuLabels::from_paths(&self.selected_paths()).copy_as_pathname,
                        self.selection_count(),
                        can_open_with,
                        self.open_with_menu
                            .as_ref()
                            .and_then(|(path, association)| {
                                (self.selected_paths().first() == Some(path)).then_some(association)
                            }),
                        can_paste,
                        self.trash_view,
                        self.applications_view,
                        undo_label,
                        self.selected_tag_checks(),
                        self.file_words,
                    ),
                    MenuPurpose::Sort => Self::build_sort_menu(state.position(), sort_key),
                    MenuPurpose::Sidebar => Self::build_sidebar_menu(
                        state.position(),
                        self.sidebar_context_is_favourite,
                        self.sidebar_context_path
                            .as_ref()
                            .is_some_and(|path| path.as_os_str().is_empty()),
                    ),
                    MenuPurpose::TitlePath => self.build_title_path_menu(state.position()),
                };
                el.child(menu.render(&state))
            })
            .when_some(conflict_dialog, |el, dialog| el.child(dialog))
            .when_some(recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(trash_recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
            .when_some(open_with_dialog, |el, dialog| el.child(dialog))
            .when_some(archive_sheet, |el, sheet| el.child(sheet))
            .when_some(archive_alert, |el, dialog| el.child(dialog))
            .when_some(rename_alert, |el, dialog| el.child(dialog))
            .when_some(help_dialog, |el, dialog| el.child(dialog))
            .when_some(missing_favourite, |el, dialog| el.child(dialog))
    }
}

impl FinderView {
    pub(super) fn publish_app_menu_state(&self, cx: &mut Context<Self>) {
        // These labels belong to the one shared application menu, so derive
        // them from the active window's current selection alongside the
        // other window-specific menu state.
        let selection = if self.trash_view || self.applications_view {
            Vec::new()
        } else {
            self.selected_paths()
        };
        let labels = SelectionMenuLabels::from_paths(&selection);
        let compress_label = archive_controller::compress_menu_label(&selection);
        rmac_ui::set_menu_label("finder::CopyItems", &labels.copy, cx);
        rmac_ui::set_menu_label("finder::CopyAsPathname", &labels.copy_as_pathname, cx);
        let quick_look_label = match selection.as_slice() {
            [path] => {
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                format!("Quick Look “{}”", sanitize_dialog_name(&name))
            }
            paths if paths.len() > 1 => format!("Quick Look {} Items", paths.len()),
            _ => "Quick Look".to_owned(),
        };
        rmac_ui::set_menu_label("finder::QuickLook", &quick_look_label, cx);
        let has_selection = !selection.is_empty();
        rmac_ui::set_menu_enabled(
            "finder::Eject",
            self.selected_ejectable_volume().is_some(),
            cx,
        );
        rmac_ui::set_menu_enabled("finder::QuickLook", has_selection, cx);
        rmac_ui::set_menu_enabled(
            "finder::GoShared",
            rmac_finder::places::shared_folder(&self.home).is_some(),
            cx,
        );
        rmac_ui::set_menu_checked(
            "finder::UseGroups",
            self.current_options().group_by != view_options::GroupBy::None,
            cx,
        );
        for (action, mode) in [
            ("finder::ViewAsIcons", ViewMode::Icon),
            ("finder::ViewAsList", ViewMode::List),
            ("finder::ViewAsColumns", ViewMode::Column),
            ("finder::ViewAsGallery", ViewMode::Gallery),
        ] {
            rmac_ui::set_menu_checked(action, self.view == mode, cx);
        }
        for action in [
            "finder::CopyAsPathname",
            "finder::CopyAsLink",
            "finder::DeselectAll",
        ] {
            rmac_ui::set_menu_enabled(action, has_selection, cx);
        }
        let has_folder = self.selected_folder().is_some();
        for action in [
            "finder::OpenSelectionInNewTab",
            "finder::OpenSelectionInNewWindowAndClose",
        ] {
            rmac_ui::set_menu_enabled(action, has_folder, cx);
        }
        rmac_ui::set_menu_enabled("finder::GoUpInNewWindow", self.cwd.parent().is_some(), cx);
        rmac_ui::set_menu_label(
            "finder::UndoOperation",
            self.undo_available
                .as_ref()
                .map_or("Undo", |available| available.label.as_str()),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "finder::UndoOperation",
            self.undo_available.is_some() && self.undo_operation.is_none(),
            cx,
        );
        #[cfg(any(target_os = "linux", test))]
        let empty_bin_available = self.trash_store.is_some() && self.trash_operation.is_none();
        #[cfg(not(any(target_os = "linux", test)))]
        let empty_bin_available = false;
        rmac_ui::set_menu_enabled("finder::EmptyTrashImmediately", empty_bin_available, cx);
        rmac_ui::set_menu_label(
            "finder::Compress",
            compress_label.as_deref().unwrap_or("Compress"),
            cx,
        );

        let state = FinderMenuState::new(
            self.tabs.len(),
            self.sidebar_visible,
            self.show_path_bar,
            self.sort_key,
            self.search_relevance_order,
        );
        rmac_ui::set_menu_label("finder::CloseTab", state.close_label, cx);
        rmac_ui::set_menu_label("finder::ToggleSidebar", state.sidebar_label, cx);
        rmac_ui::set_menu_label("finder::TogglePathBar", state.path_bar_label, cx);
        rmac_ui::set_menu_label(
            "finder::ToggleTabBar",
            if self.show_tab_bar || self.tabs.len() > 1 {
                "Hide Tab Bar"
            } else {
                "Show Tab Bar"
            },
            cx,
        );
        rmac_ui::set_menu_enabled("finder::ToggleTabBar", self.tabs.len() == 1, cx);
        rmac_ui::set_menu_label(
            "finder::ToggleStatusBar",
            if self.show_status_bar {
                "Hide Status Bar"
            } else {
                "Show Status Bar"
            },
            cx,
        );
        for (action, checked) in [
            ("finder::SortByName", state.sort_name),
            ("finder::SortByDate", state.sort_date),
            ("finder::SortBySize", state.sort_size),
            ("finder::SortByKind", state.sort_kind),
        ] {
            rmac_ui::set_menu_checked(action, checked, cx);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SelectionMenuLabels {
    copy: String,
    copy_as_pathname: String,
}

impl SelectionMenuLabels {
    fn from_paths(paths: &[std::path::PathBuf]) -> Self {
        match paths {
            [] => Self {
                copy: "Copy".into(),
                copy_as_pathname: "Copy as Pathname".into(),
            },
            [path] => {
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                let quoted = format!("“{}”", sanitize_dialog_name(&name));
                Self {
                    copy: format!("Copy {quoted}"),
                    copy_as_pathname: format!("Copy {quoted} as Pathname"),
                }
            }
            _ => Self {
                copy: format!("Copy {} Items", paths.len()),
                copy_as_pathname: format!("Copy {} Items as Pathname", paths.len()),
            },
        }
    }
}

struct FinderMenuState {
    close_label: &'static str,
    sidebar_label: &'static str,
    path_bar_label: &'static str,
    sort_name: bool,
    sort_date: bool,
    sort_size: bool,
    sort_kind: bool,
}

impl FinderMenuState {
    fn new(
        tab_count: usize,
        sidebar_visible: bool,
        show_path_bar: bool,
        sort_key: SortKey,
        relevance_order: bool,
    ) -> Self {
        Self {
            close_label: if tab_count > 1 {
                "Close Tab"
            } else {
                "Close Window"
            },
            sidebar_label: if sidebar_visible {
                "Hide Sidebar"
            } else {
                "Show Sidebar"
            },
            path_bar_label: if show_path_bar {
                "Hide Path Bar"
            } else {
                "Show Path Bar"
            },
            sort_name: !relevance_order && sort_key == SortKey::Name,
            sort_date: !relevance_order && sort_key == SortKey::Date,
            sort_size: !relevance_order && sort_key == SortKey::Size,
            sort_kind: !relevance_order && sort_key == SortKey::Kind,
        }
    }
}

#[cfg(test)]
mod app_menu_tests {
    use super::*;

    #[test]
    fn live_menu_state_tracks_tabs_toggles_and_effective_sort() {
        let state = FinderMenuState::new(1, true, false, SortKey::Name, false);
        assert_eq!(state.close_label, "Close Window");
        assert_eq!(state.sidebar_label, "Hide Sidebar");
        assert_eq!(state.path_bar_label, "Show Path Bar");
        assert_eq!(
            (
                state.sort_name,
                state.sort_date,
                state.sort_size,
                state.sort_kind
            ),
            (true, false, false, false)
        );

        let state = FinderMenuState::new(2, false, true, SortKey::Date, false);
        assert_eq!(state.close_label, "Close Tab");
        assert_eq!(state.sidebar_label, "Show Sidebar");
        assert_eq!(state.path_bar_label, "Hide Path Bar");
        assert_eq!(
            (
                state.sort_name,
                state.sort_date,
                state.sort_size,
                state.sort_kind
            ),
            (false, true, false, false)
        );

        let state = FinderMenuState::new(2, false, true, SortKey::Date, true);
        assert_eq!(
            (
                state.sort_name,
                state.sort_date,
                state.sort_size,
                state.sort_kind
            ),
            (false, false, false, false)
        );
    }

    #[test]
    fn selection_menu_labels_follow_finder_selection() {
        assert_eq!(
            SelectionMenuLabels::from_paths(&[]),
            SelectionMenuLabels {
                copy: "Copy".into(),
                copy_as_pathname: "Copy as Pathname".into(),
            }
        );
        assert_eq!(
            SelectionMenuLabels::from_paths(&["/Users/me/test.txt".into()]),
            SelectionMenuLabels {
                copy: "Copy “test.txt”".into(),
                copy_as_pathname: "Copy “test.txt” as Pathname".into(),
            }
        );
        assert_eq!(
            SelectionMenuLabels::from_paths(&[
                "/Users/me/one.txt".into(),
                "/Users/me/two.txt".into(),
            ]),
            SelectionMenuLabels {
                copy: "Copy 2 Items".into(),
                copy_as_pathname: "Copy 2 Items as Pathname".into(),
            }
        );
    }
}
