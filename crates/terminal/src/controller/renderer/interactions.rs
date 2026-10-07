//! Terminal canvas plus keyboard, action, pointer, and scroll event wiring.

use super::*;

impl TerminalView {
    pub(super) fn render_terminal_body(
        &mut self,
        rows: Vec<gpui::AnyElement>,
        ime_preedit: Option<Div>,
        a11y_active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let input_view = cx.entity().clone();
        let input_focus = self.focus.clone();
        let input_bridge = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                window.handle_input(
                    &input_focus,
                    ElementInputHandler::new(bounds, input_view),
                    cx,
                );
            },
        )
        .absolute()
        .inset_0();

        div()
            .id("terminal-grid")
            .role(Role::Terminal)
            .aria_label("Terminal")
            .a11y_synthetic_children(self.render_terminal_accessibility(a11y_active, cx))
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.modal_open() {
                    if event.keystroke.key == "escape" {
                        if this.pending_close.is_some() {
                            this.cancel_close(window, cx);
                        } else {
                            this.cancel_paste(window, cx);
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
                this.wake_cursor_blink(window, cx);
                match this.on_key_down(event) {
                    Ok(false) => return,
                    Ok(true) => {}
                    Err(error) => {
                        if error == SessionWriteError::State {
                            this.operation_error = Some(error.to_string().into());
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if this.modal_open() {
                    return;
                }
                match this.on_key_up(event) {
                    Ok(false) => return,
                    Ok(true) => {}
                    Err(error) => {
                        if error == SessionWriteError::State {
                            this.operation_error = Some(error.to_string().into());
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &CopyPlainText, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &CopyWithoutBackgroundColour, _, cx| {
                this.copy_without_background_colour(cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenManPageForSelection, window, cx| {
                    this.man_page_for_selection(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SearchManPageIndexForSelection, window, cx| {
                    this.man_page_for_selection(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Paste, window, cx| this.request_paste(window, cx)))
            .on_action(
                cx.listener(|this, _: &PasteSelection, window, cx| {
                    this.paste_selection(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &PasteEscapedText, window, cx| {
                this.paste_escaped_text(window, cx)
            }))
            .on_action(cx.listener(|this, _: &PasteEscapedSelection, window, cx| {
                this.paste_escaped_selection(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.toggle_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_step(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.find_step(false, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                let size = this.font_size + 1.0;
                this.set_font(size, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                let size = this.font_size - 1.0;
                this.set_font(size, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                this.set_font(FONT_SIZE, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &Clear, _, cx| this.clear(cx)))
            .on_action(cx.listener(|this, _: &ClearScreen, _, cx| this.clear_screen(cx)))
            .on_action(cx.listener(|this, _: &ClearScrollback, _, cx| this.clear_scrollback(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleOptionAsMeta, _, cx| this.toggle_option_as_meta(cx)),
            )
            .on_action(
                cx.listener(|this, _: &HideFindBar, window, cx| this.hide_find_bar(window, cx)),
            )
            .on_action(cx.listener(|this, _: &UseSelectionForFind, window, cx| {
                this.use_selection_for_find(window, cx)
            }))
            .on_action(cx.listener(|this, _: &JumpToSelection, _, cx| this.jump_to_selection(cx)))
            .on_action(
                cx.listener(|this, _: &UseSettingsAsDefault, _, cx| {
                    this.use_settings_as_default(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ExportSettings, window, cx| {
                    this.export_settings(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ExportTextAs, window, cx| this.export_text_as(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExportSelectedTextAs, window, cx| {
                this.export_selected_text_as(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Print, window, cx| this.print(window, cx)))
            .on_action(
                cx.listener(|this, _: &PrintSelection, window, cx| {
                    this.print_selection(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ScrollToTop, _, cx| this.scroll_view(Scroll::Top, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ScrollToBottom, _, cx| this.scroll_view(Scroll::Bottom, cx)),
            )
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll_view(Scroll::PageUp, cx)))
            .on_action(
                cx.listener(|this, _: &PageDown, _, cx| this.scroll_view(Scroll::PageDown, cx)),
            )
            .on_action(
                cx.listener(|this, _: &LineUp, _, cx| this.scroll_view(Scroll::Delta(1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &LineDown, _, cx| this.scroll_view(Scroll::Delta(-1), cx)),
            )
            .on_action(cx.listener(|this, _: &PreviousPrompt, _, cx| {
                this.navigate_prompt(PromptDirection::Previous, cx)
            }))
            .on_action(cx.listener(|this, _: &NextPrompt, _, cx| {
                this.navigate_prompt(PromptDirection::Next, cx)
            }))
            .on_action(cx.listener(|this, _: &Mark, _, cx| this.mark_current_line(false, cx)))
            .on_action(
                cx.listener(|this, _: &MarkAsBookmark, _, cx| this.mark_current_line(true, cx)),
            )
            .on_action(cx.listener(|this, _: &Unmark, _, cx| this.unmark_current_line(cx)))
            .on_action(
                cx.listener(|this, _: &AutomaticallyMarkPromptLines, _, cx| {
                    this.toggle_automatically_mark_prompt_lines(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &MarkLineAndSendReturn, _, cx| {
                this.mark_line_and_send_return(cx);
            }))
            .on_action(cx.listener(|this, _: &SendReturnWithoutMarking, _, cx| {
                this.send_return_without_marking(cx);
            }))
            .on_action(cx.listener(|_, _: &NoBookmarks, _, _| {}))
            .on_action(cx.listener(|this, _: &JumpToBookmark0, _, cx| this.jump_to_bookmark(0, cx)))
            .on_action(cx.listener(|this, _: &JumpToBookmark1, _, cx| this.jump_to_bookmark(1, cx)))
            .on_action(cx.listener(|this, _: &JumpToBookmark2, _, cx| this.jump_to_bookmark(2, cx)))
            .on_action(cx.listener(|this, _: &JumpToBookmark3, _, cx| this.jump_to_bookmark(3, cx)))
            .on_action(cx.listener(|this, _: &JumpToBookmark4, _, cx| this.jump_to_bookmark(4, cx)))
            .on_action(cx.listener(|this, _: &ShowMarks, _, cx| this.toggle_show_marks(cx)))
            .on_action(cx.listener(|this, _: &ShowAllTabs, _, cx| this.toggle_show_all_tabs(cx)))
            .on_action(cx.listener(|this, _: &ShowAlternativeScreen, _, cx| {
                this.set_viewing_primary_while_alt_screen(false, cx);
            }))
            .on_action(cx.listener(|this, _: &HideAlternativeScreen, _, cx| {
                this.set_viewing_primary_while_alt_screen(true, cx);
            }))
            .on_action(cx.listener(|this, _: &FindSelectAll, _, cx| {
                this.find_select_all(false, cx);
            }))
            .on_action(cx.listener(|this, _: &FindSelectAllInSelection, _, cx| {
                this.find_select_all(true, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenShell, window, cx| this.open_shell(window, cx)))
            .on_action(cx.listener(|this, _: &CancelOpenShell, window, cx| {
                this.cancel_open_shell(window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditBackgroundColour, window, cx| {
                this.open_edit_background_colour(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &CancelEditBackgroundColour, window, cx| {
                    this.cancel_edit_background_colour(window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &PreviousBookmark, _, cx| {
                this.navigate_bookmark(PromptDirection::Previous, cx)
            }))
            .on_action(cx.listener(|this, _: &NextBookmark, _, cx| {
                this.navigate_bookmark(PromptDirection::Next, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectCommand, _, cx| {
                this.select_shell_range(CommandRangeKind::Command, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectCommandOutput, _, cx| {
                this.select_shell_range(CommandRangeKind::Output, cx)
            }))
            // Shell ▸ New Window ▸ <profile>: the ⌘N row and the plain
            // "Basic" row both open a new window on the default profile,
            // under their own distinct actions, matching the Mac's own
            // duplicate rows.
            .on_action(cx.listener(|_, _: &WindowBasicDefault, _, cx| {
                open_window_with_profile("Basic", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowBasic, _, cx| {
                open_window_with_profile("Basic", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowClearDark, _, cx| {
                open_window_with_profile("Clear Dark", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowClearLight, _, cx| {
                open_window_with_profile("Clear Light", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowGrass, _, cx| {
                open_window_with_profile("Grass", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowHomebrew, _, cx| {
                open_window_with_profile("Homebrew", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowManPage, _, cx| {
                open_window_with_profile("Man Page", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowNovel, _, cx| {
                open_window_with_profile("Novel", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowOcean, _, cx| {
                open_window_with_profile("Ocean", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowPro, _, cx| {
                open_window_with_profile("Pro", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowRedSands, _, cx| {
                open_window_with_profile("Red Sands", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowSilverAerogel, _, cx| {
                open_window_with_profile("Silver Aerogel", cx);
            }))
            .on_action(cx.listener(|_, _: &WindowSolidColors, _, cx| {
                open_window_with_profile("Solid Colors", cx);
            }))
            .on_action(cx.listener(|this, _: &ResetTerminal, _, cx| this.reset(cx)))
            .on_action(cx.listener(|this, _: &HardResetTerminal, _, cx| this.hard_reset(cx)))
            // Shell ▸ New Tab ▸ <profile>: same shape as the window submenu,
            // but a tab in this window rather than a new window.
            .on_action(cx.listener(|this, _: &TabBasicDefault, window, cx| {
                this.new_tab_with_profile(profile_named("Basic"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabBasic, window, cx| {
                this.new_tab_with_profile(profile_named("Basic"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabClearDark, window, cx| {
                this.new_tab_with_profile(profile_named("Clear Dark"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabClearLight, window, cx| {
                this.new_tab_with_profile(profile_named("Clear Light"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabGrass, window, cx| {
                this.new_tab_with_profile(profile_named("Grass"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabHomebrew, window, cx| {
                this.new_tab_with_profile(profile_named("Homebrew"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabManPage, window, cx| {
                this.new_tab_with_profile(profile_named("Man Page"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabNovel, window, cx| {
                this.new_tab_with_profile(profile_named("Novel"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabOcean, window, cx| {
                this.new_tab_with_profile(profile_named("Ocean"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabPro, window, cx| {
                this.new_tab_with_profile(profile_named("Pro"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabRedSands, window, cx| {
                this.new_tab_with_profile(profile_named("Red Sands"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabSilverAerogel, window, cx| {
                this.new_tab_with_profile(profile_named("Silver Aerogel"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &TabSolidColors, window, cx| {
                this.new_tab_with_profile(profile_named("Solid Colors"), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.request_close_tab(this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.next_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                this.prev_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewWindowWithSameCommand, _, cx| {
                if let Some(exec) = this.tabs[this.active].exec_origin() {
                    open_window_with_same_command(exec, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NewTabWithSameCommand, window, cx| {
                this.new_tab_with_same_command(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CycleProfile, _, cx| {
                // ⌘⇧P toggles the profile picker.
                if !this.modal_open() {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx) {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ShowProfiles, _, cx| {
                // Right-click → Profiles… — a guaranteed mouse path to the
                // picker (the picker rows are clickable body overlays).
                if !this.modal_open() {
                    this.picker_open = true;
                    cx.notify();
                }
            }))
            // Applications own unshifted pointer input only while the parsed
            // terminal mode requests it. Shift always preserves Terminal's
            // local selection/context-menu path.
            .on_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if this.modal_open() {
                    return;
                }
                if this.activate_hyperlink(event, cx) {
                    cx.stop_propagation();
                    return;
                }
                let was_live = this.tabs[this.active].accepts_input();
                let error_before = this.operation_error.clone();
                let overlay_was_open = this.menu_at.is_some() || this.picker_open;
                if this.report_mouse_down(event) {
                    this.selecting = false;
                    this.menu_at = None;
                    this.picker_open = false;
                    if was_live != this.tabs[this.active].accepts_input()
                        || error_before != this.operation_error
                        || overlay_was_open
                    {
                        cx.notify();
                    }
                    return;
                }
                if this.picker_open {
                    this.picker_open = false;
                }
                match event.button {
                    MouseButton::Left => {
                        let offset = this.display_offset();
                        let cell = this.pos_to_cell(event.position, offset);
                        this.tabs[this.active].ui.selection = Some(Selection {
                            anchor: cell,
                            head: cell,
                        });
                        // A fresh drag-selection replaces Find ▸ Select
                        // All's set, the same way starting one elsewhere
                        // replaces the single-range selection above.
                        this.tabs[this.active].ui.selected_matches.clear();
                        this.selecting = true;
                    }
                    MouseButton::Right => {
                        this.menu_at = Some(rmac_ui::ContextMenuState::open(
                            event.position,
                            &this.focus,
                            window,
                            cx,
                        ));
                    }
                    MouseButton::Middle | MouseButton::Navigate(_) => {}
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let hyperlink_changed = this.update_hovered_link(event.position);
                if this.selecting {
                    let offset = this.display_offset();
                    let cell = this.pos_to_cell(event.position, offset);
                    if let Some(selection) = this.tabs[this.active].ui.selection.as_mut() {
                        selection.head = cell;
                    }
                    cx.notify();
                    return;
                }
                let was_live = this.tabs[this.active].accepts_input();
                let error_before = this.operation_error.clone();
                let reporting_changed = this.report_mouse_motion(event)
                    && (was_live != this.tabs[this.active].accepts_input()
                        || error_before != this.operation_error);
                if hyperlink_changed || reporting_changed {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            // GPUI dispatches an outside release separately. Handling both
            // paths prevents a drag from leaving either the terminal
            // application or local selection in a stuck state.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.handle_mouse_up(event, cx);
                }),
            )
            // Mouse-aware applications receive bounded wheel reports;
            // otherwise the wheel walks local scrollback.
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let was_live = this.tabs[this.active].accepts_input();
                let error_before = this.operation_error.clone();
                if this.report_mouse_wheel(event) {
                    if was_live != this.tabs[this.active].accepts_input()
                        || error_before != this.operation_error
                    {
                        cx.notify();
                    }
                    return;
                }
                let delta_y = match event.delta {
                    ScrollDelta::Lines(point) => point.y,
                    ScrollDelta::Pixels(point) => f32::from(point.y) / this.line_h,
                };
                this.scroll_accum += delta_y;
                let lines = this.scroll_accum.trunc() as i32;
                this.scroll_accum -= lines as f32;
                if lines != 0 {
                    this.scroll_lines(lines);
                    cx.notify();
                }
            }))
            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.85))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.drop_paths(paths.paths(), cx);
            }))
            .flex_1()
            .relative()
            .pl(px(PAD_X))
            .pr(px(PAD_X))
            .pt(px(PAD_TOP))
            .pb(px(PAD_BOTTOM))
            .bg(hsla(active().bg))
            .font_family(rmac_ui::MONO_FONT)
            .text_size(px(self.font_size))
            // Without an explicit line height, text defaults to GPUI's
            // phi()-ratio line box (about 1.618 × font size), taller than a
            // cell row's fixed 14 pt height (`self.line_h`). The excess sinks
            // past the row's bottom edge, where the next row's opaque
            // per-cell background then paints over it, clipping descenders
            // (UIA-01: "jacob" reads "iacob", "repo" reads "reoo").
            .line_height(px(self.line_h))
            .v_flex()
            .children(rows)
            .when_some(self.render_inactive_cursor(), |body, cursor| {
                body.child(cursor)
            })
            .when_some(self.render_active_cursor_overlay(), |body, cursor| {
                body.child(cursor)
            })
            .when_some(ime_preedit, |body, preedit| body.child(preedit))
            .child(input_bridge)
    }
}
