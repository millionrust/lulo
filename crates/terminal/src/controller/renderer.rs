mod accessibility;
mod chrome;
mod dialogs;
mod grid;
mod interactions;
mod overlays;

use super::*;

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiles::set_active(self.profile);
        profiles::set_active_background_override(self.background_override);
        // Settings ▸ Shell ▸ "When the shell exits": never empties `tabs`
        // (the single-tab case closes the whole *window* instead, leaving
        // `tabs` untouched while GPUI tears it down), so `self.active` stays
        // valid for the rest of this render either way.
        self.auto_close_exited_tabs(window, cx);
        self.resize_to(window);
        // Settings ▸ General ▸ "New windows open with: Same Working
        // Directory" (⌘N) tracks whichever window was frontmost; only a
        // focused window's directory is a candidate.
        if self.window_active {
            rmac_ui::set_menu_label(
                "terminal::EnterFullScreen",
                if window.is_fullscreen() {
                    "Exit Full Screen"
                } else {
                    "Enter Full Screen"
                },
                cx,
            );
            rmac_ui::set_menu_label(
                "terminal::CloseTab",
                if self.tabs.len() > 1 {
                    "Close Tab"
                } else {
                    "Close Window"
                },
                cx,
            );
            if let Some(directory) = self.tabs[self.active].working_directory() {
                crate::working_directory::set_last_front_directory(directory);
            }
            let selection = self.selection_text();
            // Copy/Export/Print: either kind of "selected text" counts —
            // the plain drag-selection, or Find ▸ Select All's matches.
            let has_selection = self
                .any_selection_text()
                .is_some_and(|text| !text.is_empty());
            let has_man_topic = selection
                .as_ref()
                .is_some_and(|text| super::input::man_command(text, false).is_some());
            rmac_ui::set_menu_enabled("terminal::Copy", has_selection, cx);
            rmac_ui::set_menu_checked("terminal::ToggleOptionAsMeta", self.option_as_meta, cx);
            rmac_ui::set_menu_checked("terminal::ShowTabBar", self.tab_bar_visible(), cx);
            rmac_ui::set_menu_checked(
                "terminal::AllowMouseReporting",
                self.allow_mouse_reporting,
                cx,
            );
            publish_copy_style_menu_state(cx);
            rmac_ui::set_menu_enabled("terminal::CopyPlainText", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::CopyWithoutBackgroundColour", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::ExportSelectedTextAs", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::PrintSelection", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::OpenManPageForSelection", has_man_topic, cx);
            rmac_ui::set_menu_enabled(
                "terminal::SearchManPageIndexForSelection",
                has_man_topic,
                cx,
            );
            rmac_ui::set_menu_enabled("terminal::PasteSelection", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::PasteEscapedSelection", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::UseSelectionForFind", has_selection, cx);
            rmac_ui::set_menu_enabled("terminal::JumpToSelection", has_selection, cx);
            let can_mark = self.tabs[self.active].can_mark_current_line();
            rmac_ui::set_menu_enabled("terminal::Mark", can_mark, cx);
            rmac_ui::set_menu_enabled("terminal::MarkAsBookmark", can_mark, cx);
            rmac_ui::set_menu_enabled(
                "terminal::Unmark",
                self.tabs[self.active].current_line_is_marked(),
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::PreviousBookmark",
                self.tabs[self.active].can_scroll_to_bookmark(PromptDirection::Previous),
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::NextBookmark",
                self.tabs[self.active].can_scroll_to_bookmark(PromptDirection::Next),
                cx,
            );
            for (action, direction, bookmark_only) in [
                (
                    "terminal::SelectToPreviousMark",
                    PromptDirection::Previous,
                    false,
                ),
                ("terminal::SelectToNextMark", PromptDirection::Next, false),
                (
                    "terminal::SelectToPreviousBookmark",
                    PromptDirection::Previous,
                    true,
                ),
                (
                    "terminal::SelectToNextBookmark",
                    PromptDirection::Next,
                    true,
                ),
            ] {
                rmac_ui::set_menu_enabled(
                    action,
                    self.tabs[self.active].can_select_to_mark(direction, bookmark_only),
                    cx,
                );
            }
            rmac_ui::set_menu_enabled(
                "terminal::HideFindBar",
                self.tabs[self.active].ui.search_open,
                cx,
            );
            // Shell ▸ New Window/Tab with Same Command: only a session that
            // is itself execing a specific program (`-e`, or a same-command
            // relaunch) has a command to repeat. A plain interactive shell
            // leaves both disabled, as the Mac does when there's nothing to
            // name.
            let has_exec_origin = self.tabs[self.active].exec_origin().is_some();
            rmac_ui::set_menu_enabled("terminal::NewWindowWithSameCommand", has_exec_origin, cx);
            rmac_ui::set_menu_enabled("terminal::NewTabWithSameCommand", has_exec_origin, cx);
            rmac_ui::set_menu_label(
                "terminal::ShowInspector",
                if self.inspector_open {
                    "Hide Inspector"
                } else {
                    "Show Inspector"
                },
                cx,
            );
            let split_active = self.tabs[self.active].ui.split_offset.is_some();
            rmac_ui::set_menu_enabled("terminal::SplitPane", !split_active, cx);
            rmac_ui::set_menu_enabled("terminal::CloseSplitPane", split_active, cx);
            rmac_ui::set_menu_enabled(
                "terminal::ClearToPreviousMark",
                self.tabs[self.active].can_clear_to_mark(false),
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::ClearToPreviousBookmark",
                self.tabs[self.active].can_clear_to_mark(true),
                cx,
            );
            // Edit ▸ Marks (TERM-16/TERM-23).
            rmac_ui::set_menu_checked(
                "terminal::AutomaticallyMarkPromptLines",
                self.automatically_mark_prompt_lines,
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::MarkLineAndSendReturn",
                can_mark && self.tabs[self.active].accepts_input(),
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::SendReturnWithoutMarking",
                self.tabs[self.active].accepts_input(),
                cx,
            );
            // Edit ▸ Bookmarks ▸: a fresh list every time the menu is
            // about to open, the same convention `rmac_app_menu::recent`
            // uses for File ▸ Open Recent.
            let bookmark_lines = self.bookmark_menu_lines();
            let bookmark_items: Vec<rmac_ui::MenuItem> = bookmark_lines
                .iter()
                .take(BOOKMARK_MENU_SLOTS)
                .enumerate()
                .map(|(index, line)| {
                    rmac_ui::MenuItem::new(
                        format!("Bookmark at line {line}"),
                        format!("terminal::JumpToBookmark{index}"),
                        "",
                    )
                })
                .collect();
            if bookmark_items.is_empty() {
                rmac_ui::set_menu_children(
                    "terminal::BookmarksMenu",
                    vec![
                        rmac_ui::MenuItem::new("No Bookmarks", "terminal::NoBookmarks", "")
                            .enabled(false),
                    ],
                    cx,
                );
            } else {
                rmac_ui::set_menu_children("terminal::BookmarksMenu", bookmark_items, cx);
            }
            // View (TERM-23).
            rmac_ui::set_menu_enabled("terminal::ShowAllTabs", self.tabs.len() > 1, cx);
            let has_any_marks = self.tabs[self.active].has_any_marks();
            rmac_ui::set_menu_enabled("terminal::ShowMarks", has_any_marks, cx);
            rmac_ui::set_menu_checked("terminal::ShowMarks", self.show_marks, cx);
            let in_alt_screen = self.active_tab_in_alt_screen();
            rmac_ui::set_menu_enabled(
                "terminal::ShowAlternativeScreen",
                in_alt_screen && self.viewing_primary_while_alt_screen,
                cx,
            );
            rmac_ui::set_menu_enabled(
                "terminal::HideAlternativeScreen",
                in_alt_screen && !self.viewing_primary_while_alt_screen,
                cx,
            );
            // Edit ▸ Find ▸ Select All/Select All in Selection: only
            // meaningful with a query typed and, for "in Selection", an
            // existing single-range selection to search within.
            let has_query = !self.tabs[self.active].ui.search_query.is_empty();
            rmac_ui::set_menu_enabled("terminal::FindSelectAll", has_query, cx);
            rmac_ui::set_menu_enabled(
                "terminal::FindSelectAllInSelection",
                has_query && self.tabs[self.active].ui.selection.is_some(),
                cx,
            );
        }
        let layout = responsive_layout::terminal_layout(f32::from(
            rmac_ui::window_content_size(window).width,
        ));
        let raw_query = self.search.read(cx).value().to_string();
        let bounded_query = bounded_search_query(&raw_query);
        if bounded_query != raw_query {
            let normalized = bounded_query.clone();
            self.search
                .update(cx, |state, cx| state.set_value(normalized, window, cx));
        }
        self.tabs[self.active].ui.search_query = bounded_query.clone();
        let searching = self.tabs[self.active].ui.search_open;
        let query = if searching {
            bounded_query.to_lowercase()
        } else {
            String::new()
        };
        let rows = self.render_rows(&query);
        let split_pane = self
            .render_split_pane(&query, cx)
            .map(|pane| pane.into_any_element());
        let active_title = self.tabs[self.active]
            .tab_title()
            .unwrap_or_else(|| "Terminal".into());
        let native_window_title = rmac_ui::native_window_title(&active_title, "Terminal");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        let show_tab_bar = self.tab_bar_visible();
        let operation_error_visible = self.operation_error.is_some();
        let terminal_error = self
            .operation_error
            .clone()
            .or_else(|| self.persistence_error.clone());
        let has_terminal_error = terminal_error.is_some();
        let session_status = self.tabs[self.active]
            .status_message()
            .map(SharedString::from);
        let hyperlink_status = session_status
            .is_none()
            .then(|| self.hovered_link.clone())
            .flatten();
        let close_confirmation = self
            .render_close_confirmation(cx)
            .map(|alert| alert.into_any_element());
        let paste_confirmation = self
            .render_paste_confirmation(cx)
            .map(|alert| alert.into_any_element());
        let new_command_sheet = self
            .render_new_command(cx)
            .map(|sheet| sheet.into_any_element());
        let new_remote_connection_sheet = self
            .render_new_remote_connection(cx)
            .map(|sheet| sheet.into_any_element());
        let edit_title_sheet = self
            .render_edit_title(cx)
            .map(|sheet| sheet.into_any_element());
        let open_shell_sheet = self
            .render_open_shell(cx)
            .map(|sheet| sheet.into_any_element());
        let background_colour_sheet = self
            .render_edit_background_colour(cx)
            .map(|sheet| sheet.into_any_element());
        let all_tabs_overlay = self
            .render_all_tabs(cx)
            .map(|overlay| overlay.into_any_element());
        let inspector = self
            .render_inspector()
            .map(|panel| panel.into_any_element());
        let ime_preedit = (!searching && !self.modal_open())
            .then(|| self.render_ime_preedit())
            .flatten();

        div()
            .size_full()
            .relative()
            .v_flex()
            .bg(hsla(active().bg))
            // The red traffic light dispatches `RequestClose` from
            // whichever element currently holds focus (the grid, the Find
            // field, …), and GPUI bubbles it up the render tree from
            // there. Handling it here on the true root — not on
            // `#terminal-grid`, which is only a sibling of the title bar,
            // Find panel and pickers — keeps the close button working no
            // matter what has focus, mirroring Text Editor/Notes/Finder.
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.request_close_window(window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                this.request_close_window(window, cx)
            }))
            .on_action(cx.listener(|_, _: &CloseAll, _, cx| {
                // Dispatch after the current action returns; a render-tree
                // handler cannot recursively update its own window.
                cx.defer(|cx| {
                    for handle in cx.windows() {
                        let _ = handle.update(cx, |_, window, cx| {
                            window.dispatch_action(Box::new(CloseWindow), cx);
                        });
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.show_settings(cx)))
            .on_action(cx.listener(|this, _: &ShowTabBar, _, cx| this.toggle_tab_bar(cx)))
            .on_action(cx.listener(|this, _: &AllowMouseReporting, _, cx| {
                this.allow_mouse_reporting = !this.allow_mouse_reporting;
                this.reset_pointer_routing();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &EnterFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &SelectToPreviousMark, _, cx| {
                this.select_to_mark(PromptDirection::Previous, false, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToNextMark, _, cx| {
                this.select_to_mark(PromptDirection::Next, false, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToPreviousBookmark, _, cx| {
                this.select_to_mark(PromptDirection::Previous, true, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToNextBookmark, _, cx| {
                this.select_to_mark(PromptDirection::Next, true, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearToPreviousMark, _, cx| {
                this.clear_to_previous_mark(false, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearToPreviousBookmark, _, cx| {
                this.clear_to_previous_mark(true, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NewCommand, window, cx| this.open_new_command(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NewRemoteConnection, window, cx| {
                this.open_new_remote_connection(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &EditTitle, window, cx| this.open_edit_title(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowInspector, _, cx| this.toggle_inspector(cx)))
            .on_action(cx.listener(|this, _: &SplitPane, _, cx| this.toggle_split_pane(cx)))
            .on_action(cx.listener(|this, _: &CloseSplitPane, _, cx| this.close_split_pane(cx)))
            .child(rmac_ui::title_bar_content(
                self.render_title(active_title, layout.title_max_width),
            ))
            .when(show_tab_bar, |terminal: Div| {
                terminal.child(self.render_tabs(layout.tab_title_max_width, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .v_flex()
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex()
                            .overflow_hidden()
                            .child(self.render_terminal_body(
                                rows,
                                ime_preedit,
                                window.is_a11y_active(),
                                cx,
                            )),
                    )
                    .when_some(split_pane, |column, pane| column.child(pane)),
            )
            .when(searching, |terminal| {
                terminal.child(self.render_find_panel(layout.find_width, cx))
            })
            // The picker is an absolute overlay — render it LAST so it paints on
            // top of the opaque terminal body instead of behind it.
            .when(self.picker_open, |terminal: Div| {
                terminal.child(self.render_picker(cx))
            })
            // The right-click context menu paints above everything else.
            .when_some(self.menu_at.clone(), |terminal: Div, state| {
                terminal.child(self.render_context_menu(state))
            })
            .when_some(session_status, |terminal, message| {
                terminal.child(self.render_session_status(message, has_terminal_error, cx))
            })
            .when_some(hyperlink_status, |terminal, message| {
                terminal.child(self.render_hyperlink_status(message, has_terminal_error))
            })
            .when_some(terminal_error, |terminal, message| {
                terminal.child(self.render_terminal_error(message, operation_error_visible, cx))
            })
            // Shell ▸ Show Inspector is a non-modal panel, so it paints
            // under the modal sheets/reviews below rather than after them.
            .when_some(inspector, |terminal, panel| terminal.child(panel))
            // Modal reviews remain the final children so no terminal surface
            // can paint over them or receive pointer input.
            .when_some(paste_confirmation, |terminal, alert| terminal.child(alert))
            .when_some(close_confirmation, |terminal, alert| terminal.child(alert))
            .when_some(new_command_sheet, |terminal, sheet| terminal.child(sheet))
            .when_some(new_remote_connection_sheet, |terminal, sheet| {
                terminal.child(sheet)
            })
            .when_some(edit_title_sheet, |terminal, sheet| terminal.child(sheet))
            .when_some(open_shell_sheet, |terminal, sheet| terminal.child(sheet))
            .when_some(background_colour_sheet, |terminal, sheet| {
                terminal.child(sheet)
            })
            // View ▸ Show All Tabs paints over everything else in the
            // window, like Preview/Text Editor's own Exposé-style grid.
            .when_some(all_tabs_overlay, |terminal, overlay| {
                terminal.child(overlay)
            })
    }
}

fn hsla(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}
