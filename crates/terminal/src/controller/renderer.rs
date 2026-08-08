mod chrome;
mod dialogs;
mod grid;

use super::*;

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiles::set_active(self.profile);
        self.resize_to(window);
        let layout = responsive_layout::terminal_layout(f32::from(window.bounds().size.width));
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
        let active_title = self.tabs[self.active]
            .tab_title()
            .unwrap_or_else(|| "Terminal".into());
        let native_window_title = rmac_ui::native_window_title(&active_title, "Terminal");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        let multi = self.tabs.len() > 1;
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
        let ime_preedit = (!searching && !self.modal_open())
            .then(|| self.render_ime_preedit())
            .flatten();
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
            .size_full()
            .relative()
            .v_flex()
            .bg(hsla(active().bg))
            .child(rmac_ui::toolbar(
                // Three flex sections: a left spacer balances the right chip so
                // "Terminal" stays centered. No absolute positioning — that broke
                // click hit-testing for the chip inside the TitleBar.
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .child(div().flex_1())
                    .child(
                        div()
                            .max_w(px(layout.title_max_width))
                            .truncate()
                            .child(active_title),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_end()
                            .pr_2()
                            .child(self.profile_chip(cx)),
                    ),
            ))
            .when(multi, |el: Div| {
                el.child(self.render_tabs(layout.tab_title_max_width, cx))
            })
            .child(
                div()
                    .track_focus(&self.focus)
                    .key_context("Terminal")
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if this.modal_open() {
                            if ev.keystroke.key == "escape" {
                                if this.pending_close.is_some() {
                                    this.cancel_close(window, cx);
                                } else {
                                    this.cancel_paste(window, cx);
                                }
                            }
                            cx.stop_propagation();
                            return;
                        }
                        match this.on_key_down(ev) {
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
                    .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                        if this.modal_open() {
                            return;
                        }
                        match this.on_key_up(ev) {
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
                    .on_action(
                        cx.listener(|this, _: &Paste, window, cx| this.request_paste(window, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &Find, window, cx| this.toggle_find(window, cx)),
                    )
                    .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                        let s = this.font_size + 1.0;
                        this.set_font(s, cx);
                    }))
                    .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                        let s = this.font_size - 1.0;
                        this.set_font(s, cx);
                    }))
                    .on_action(
                        cx.listener(|this, _: &ZoomReset, _, cx| this.set_font(FONT_SIZE, cx)),
                    )
                    .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
                    .on_action(cx.listener(|this, _: &Clear, _, cx| this.clear(cx)))
                    .on_action(cx.listener(|this, _: &PreviousPrompt, _, cx| {
                        this.navigate_prompt(PromptDirection::Previous, cx)
                    }))
                    .on_action(cx.listener(|this, _: &NextPrompt, _, cx| {
                        this.navigate_prompt(PromptDirection::Next, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectCommand, _, cx| {
                        this.select_shell_range(CommandRangeKind::Command, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectCommandOutput, _, cx| {
                        this.select_shell_range(CommandRangeKind::Output, cx)
                    }))
                    .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                        this.new_tab(window, cx);
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
                    .on_action(cx.listener(|this, _: &CycleProfile, _, cx| {
                        // ⌘⇧P toggles the profile picker.
                        if !this.modal_open() {
                            this.picker_open = !this.picker_open;
                            cx.notify();
                        }
                    }))
                    .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                        if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window) {
                            cx.notify();
                        }
                    }))
                    .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                        this.request_close_window(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ShowProfiles, _, cx| {
                        // Right-click → Profiles… — a guaranteed mouse path to the
                        // picker (the picker rows are clickable body overlays).
                        if !this.modal_open() {
                            this.picker_open = true;
                            cx.notify();
                        }
                    }))
                    // Applications own unshifted pointer input only while the
                    // parsed terminal mode requests it. Shift always preserves
                    // Terminal's local selection/context-menu path.
                    .on_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        if this.modal_open() {
                            return;
                        }
                        if this.activate_hyperlink(ev, cx) {
                            cx.stop_propagation();
                            return;
                        }
                        let was_live = this.tabs[this.active].accepts_input();
                        let error_before = this.operation_error.clone();
                        let overlay_was_open = this.menu_at.is_some() || this.picker_open;
                        if this.report_mouse_down(ev) {
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
                        match ev.button {
                            MouseButton::Left => {
                                let offset = this.display_offset();
                                let cell = this.pos_to_cell(ev.position, offset);
                                this.tabs[this.active].ui.selection = Some(Selection {
                                    anchor: cell,
                                    head: cell,
                                });
                                this.selecting = true;
                            }
                            MouseButton::Right => {
                                this.menu_at = Some(rmac_ui::ContextMenuState::open(
                                    ev.position,
                                    &this.focus,
                                    window,
                                    cx,
                                ));
                            }
                            MouseButton::Middle | MouseButton::Navigate(_) => {}
                        }
                        cx.notify();
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        let hyperlink_changed = this.update_hovered_link(ev.position);
                        if this.selecting {
                            let offset = this.display_offset();
                            let cell = this.pos_to_cell(ev.position, offset);
                            if let Some(sel) = this.tabs[this.active].ui.selection.as_mut() {
                                sel.head = cell;
                            }
                            cx.notify();
                            return;
                        }
                        let was_live = this.tabs[this.active].accepts_input();
                        let error_before = this.operation_error.clone();
                        let reporting_changed = this.report_mouse_motion(ev)
                            && (was_live != this.tabs[this.active].accepts_input()
                                || error_before != this.operation_error);
                        if hyperlink_changed || reporting_changed {
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Navigate(NavigationDirection::Back),
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Navigate(NavigationDirection::Forward),
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    // GPUI dispatches an outside release separately. Handling
                    // both paths prevents a drag from leaving either the
                    // terminal application or local selection in a stuck state.
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Middle,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Navigate(NavigationDirection::Back),
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Navigate(NavigationDirection::Forward),
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                            this.handle_mouse_up(ev, cx);
                        }),
                    )
                    // Mouse-aware applications receive bounded wheel reports;
                    // otherwise the wheel walks local scrollback.
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                        let was_live = this.tabs[this.active].accepts_input();
                        let error_before = this.operation_error.clone();
                        if this.report_mouse_wheel(ev) {
                            if was_live != this.tabs[this.active].accepts_input()
                                || error_before != this.operation_error
                            {
                                cx.notify();
                            }
                            return;
                        }
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y,
                            ScrollDelta::Pixels(p) => f32::from(p.y) / this.line_h,
                        };
                        this.scroll_accum += dy;
                        let lines = this.scroll_accum.trunc() as i32;
                        this.scroll_accum -= lines as f32;
                        if lines != 0 {
                            this.scroll_lines(lines);
                            cx.notify();
                        }
                    }))
                    .flex_1()
                    .relative()
                    .p_2()
                    .bg(hsla(active().bg))
                    .font_family(FONT)
                    .text_size(px(self.font_size))
                    .v_flex()
                    .children(rows)
                    .when_some(ime_preedit, |body, preedit| body.child(preedit))
                    .child(input_bridge),
            )
            .when(searching, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(40.0))
                        .right(px(12.0))
                        .w(px(layout.find_width))
                        .h(px(30.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .rounded(px(7.0))
                        .bg(rmac_ui::mac::raised())
                        .child(
                            div()
                                .flex_1()
                                .child(SearchField::new(&self.search).appearance(false)),
                        )
                        .child(
                            div()
                                .id("find-close")
                                .text_color(rmac_ui::mac::text_secondary())
                                .child("×")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.capture_active_search_query(cx);
                                    this.tabs[this.active].ui.search_open = false;
                                    window.focus(&this.focus);
                                    cx.notify();
                                })),
                        ),
                )
            })
            // The picker is an absolute overlay — render it LAST so it paints on
            // top of the opaque terminal body instead of behind it.
            .when(self.picker_open, |el: Div| el.child(self.render_picker(cx)))
            // The right-click context menu paints above everything else.
            .when_some(self.menu_at.clone(), |el: Div, state| {
                el.child(
                    rmac_ui::ContextMenu::new(state.position())
                        .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(Copy))
                        .command_item("Paste", rmac_ui::shortcuts::PASTE, Box::new(Paste))
                        .command_item(
                            "Select All",
                            rmac_ui::shortcuts::SELECT_ALL,
                            Box::new(SelectAll),
                        )
                        .separator()
                        .command_item(
                            "Previous Prompt",
                            rmac_ui::shortcuts::PREVIOUS_MARK,
                            Box::new(PreviousPrompt),
                        )
                        .command_item(
                            "Next Prompt",
                            rmac_ui::shortcuts::NEXT_MARK,
                            Box::new(NextPrompt),
                        )
                        .item("Select Command", Box::new(SelectCommand))
                        .command_item(
                            "Select Command Output",
                            rmac_ui::shortcuts::SELECT_COMMAND_OUTPUT,
                            Box::new(SelectCommandOutput),
                        )
                        .separator()
                        .command_item("Clear", rmac_ui::shortcuts::CLEAR, Box::new(Clear))
                        .separator()
                        .item("Profiles…", Box::new(ShowProfiles))
                        .render(&state),
                )
            })
            .when_some(session_status, |terminal, message| {
                terminal.child(
                    div()
                        .id("session-status")
                        .absolute()
                        .left(px(8.0))
                        .right(px(8.0))
                        .bottom(px(if has_terminal_error { 54.0 } else { 8.0 }))
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(7.0))
                        .bg(rmac_ui::mac::raised())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::text())
                        .shadow_lg()
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("new-tab-after-exit", "New Tab")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_tab(window, cx);
                                })),
                        ),
                )
            })
            .when_some(hyperlink_status, |terminal, message| {
                terminal.child(
                    div()
                        .id("hyperlink-status")
                        .absolute()
                        .left(px(8.0))
                        .bottom(px(if has_terminal_error { 54.0 } else { 8.0 }))
                        .max_w(px(460.0))
                        .px_3()
                        .py_2()
                        .rounded(px(7.0))
                        .bg(rmac_ui::mac::raised())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::text_secondary())
                        .shadow_lg()
                        .truncate()
                        .child(message),
                )
            })
            .when_some(terminal_error, |terminal, message| {
                terminal.child(
                    div()
                        .id("terminal-error")
                        .absolute()
                        .left(px(8.0))
                        .right(px(8.0))
                        .bottom(px(8.0))
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(7.0))
                        .bg(rmac_ui::mac::danger())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::on_danger())
                        .shadow_lg()
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("dismiss-terminal-error", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if operation_error_visible {
                                        this.operation_error = None;
                                    } else {
                                        this.persistence_error = None;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            })
            // Modal reviews remain the final children so no terminal surface
            // can paint over them or receive pointer input.
            .when_some(paste_confirmation, |terminal, alert| terminal.child(alert))
            .when_some(close_confirmation, |terminal, alert| terminal.child(alert))
    }
}

fn span(text: &str, style: Style) -> gpui::AnyElement {
    let mut element = div()
        .text_color(style.fg)
        .bg(style.bg)
        .child(text.to_string());
    if style.bold {
        element = element.font_weight(FontWeight::BOLD);
    }
    if style.italic {
        element = element.italic();
    }
    if style.underline {
        element = element.underline();
    }
    if style.strike {
        element = element.line_through();
    }
    element.into_any_element()
}

fn hsla(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Map a terminal color to RGB.
fn conv(color: Color) -> Hsla {
    let (red, green, blue) = match color {
        Color::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        Color::Named(named) => named_color(named),
        Color::Indexed(index) => indexed_color(index),
    };
    gpui::rgb(((red as u32) << 16) | ((green as u32) << 8) | (blue as u32)).into()
}

fn named_color(color: NamedColor) -> (u8, u8, u8) {
    use NamedColor::*;
    let profile = active();
    let ansi = |index: usize| split(profile.ansi[index]);
    match color {
        Background => split(profile.bg),
        Foreground => split(profile.fg),
        Cursor => split(profile.cursor),
        Black => ansi(0),
        Red => ansi(1),
        Green => ansi(2),
        Yellow => ansi(3),
        Blue => ansi(4),
        Magenta => ansi(5),
        Cyan => ansi(6),
        White => ansi(7),
        BrightBlack => ansi(8),
        BrightRed => ansi(9),
        BrightGreen => ansi(10),
        BrightYellow => ansi(11),
        BrightBlue => ansi(12),
        BrightMagenta => ansi(13),
        BrightCyan => ansi(14),
        BrightWhite => ansi(15),
        _ => split(profile.fg),
    }
}

fn indexed_color(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => split(active().ansi[index as usize]),
        16..=231 => {
            let index = index - 16;
            let component = |value: u8| -> u8 {
                if value == 0 {
                    0
                } else {
                    55 + 40 * value
                }
            };
            (
                component(index / 36),
                component((index % 36) / 6),
                component(index % 6),
            )
        }
        _ => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
    }
}

fn split(hex: u32) -> (u8, u8, u8) {
    (
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}
