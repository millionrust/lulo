//! Terminal canvas plus keyboard, action, pointer, and scroll event wiring.

use super::*;

impl TerminalView {
    pub(super) fn render_terminal_body(
        &self,
        rows: Vec<gpui::AnyElement>,
        ime_preedit: Option<Div>,
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
            .on_action(cx.listener(|this, _: &Paste, window, cx| this.request_paste(window, cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.toggle_find(window, cx)))
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
            .flex_1()
            .relative()
            .p_2()
            .bg(hsla(active().bg))
            .font_family(rmac_ui::MONO_FONT)
            .text_size(px(self.font_size))
            .v_flex()
            .children(rows)
            .when_some(ime_preedit, |body, preedit| body.child(preedit))
            .child(input_bridge)
    }
}
