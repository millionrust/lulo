//! Viewport geometry, local selection, and terminal mouse-protocol routing.

use super::*;

impl TerminalView {
    pub(super) fn reset_pointer_routing(&mut self) {
        self.selecting = false;
        self.scroll_accum = 0.0;
        self.mouse_wheel_x_accum = 0.0;
        self.mouse_wheel_y_accum = 0.0;
        self.reported_mouse_press = None;
        self.last_mouse_report_cell = None;
    }

    /// Current scrollback offset (0 = pinned to the live prompt).
    pub(super) fn display_offset(&self) -> i32 {
        self.tabs[self.active]
            .term
            .lock()
            .map(|term| term.grid().display_offset() as i32)
            .unwrap_or(0)
    }

    /// Convert a window-space mouse position to a `(grid_line, column)` cell.
    /// `offset` keeps history selections anchored to content.
    pub(super) fn pos_to_cell(&self, position: Point<Pixels>, offset: i32) -> (i32, usize) {
        let (row, column) = self.pos_to_viewport_cell(position);
        (row as i32 - offset, column)
    }

    pub(super) fn terminal_content_top(&self) -> f32 {
        terminal_content_top(self.tabs.len())
    }

    fn pos_to_viewport_cell(&self, position: Point<Pixels>) -> (usize, usize) {
        let x = f32::from(position.x);
        let y = f32::from(position.y);
        let column =
            (((x - LEFT_PAD) / self.cell_w).floor() as i32).clamp(0, self.cols as i32 - 1) as usize;
        let row = (((y - self.terminal_content_top()) / self.line_h).floor() as i32)
            .clamp(0, self.rows as i32 - 1) as usize;
        (row, column)
    }

    /// Scroll the viewport by `lines` (positive = into history).
    pub(super) fn scroll_lines(&mut self, lines: i32) {
        if lines == 0 {
            return;
        }
        if let Ok(mut term) = self.tabs[self.active].term.lock() {
            term.scroll_display(Scroll::Delta(lines));
        }
    }

    pub(super) fn active_terminal_mode(&self) -> Result<TermMode, SessionWriteError> {
        self.tabs[self.active]
            .term
            .lock()
            .map(|term| *term.mode())
            .map_err(|_| SessionWriteError::State)
    }

    pub(super) fn report_mouse_down(&mut self, event: &MouseDownEvent) -> bool {
        if event.modifiers.shift || !self.tabs[self.active].accepts_input() {
            return false;
        }
        let mode = match self.active_terminal_mode() {
            Ok(mode) => mode,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                return true;
            }
        };
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        let (row, column) = self.pos_to_viewport_cell(event.position);
        let Some(bytes) = encode_mouse_report(
            mode,
            MouseReport::Press(event.button),
            column,
            row,
            &event.modifiers,
        ) else {
            // The application owns unshifted input while reporting is enabled,
            // even when a legacy encoding cannot represent this large cell.
            return true;
        };
        let session_id = self.tabs[self.active].id;
        if self.tabs[self.active].write(&bytes).is_ok() {
            self.reported_mouse_press = Some((session_id, event.button));
            self.last_mouse_report_cell = Some((session_id, column, row));
        }
        self.menu_at = None;
        true
    }

    fn report_mouse_up(&mut self, event: &MouseUpEvent) -> bool {
        let active_id = self.tabs[self.active].id;
        let balanced_release = self.reported_mouse_press == Some((active_id, event.button));
        if !balanced_release && (event.modifiers.shift || !self.tabs[self.active].accepts_input()) {
            return false;
        }
        let mode = match self.active_terminal_mode() {
            Ok(mode) => mode,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                self.reported_mouse_press = None;
                return balanced_release;
            }
        };
        if !balanced_release && !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        let (row, column) = self.pos_to_viewport_cell(event.position);
        if let Some(bytes) = encode_mouse_report(
            mode,
            MouseReport::Release(event.button),
            column,
            row,
            &event.modifiers,
        ) {
            let _ = self.tabs[self.active].write(&bytes);
        }
        self.reported_mouse_press = None;
        self.last_mouse_report_cell = None;
        true
    }

    pub(super) fn handle_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let was_live = self.tabs[self.active].accepts_input();
        let error_before = self.operation_error.clone();
        if self.report_mouse_up(event) {
            self.selecting = false;
            if was_live != self.tabs[self.active].accepts_input()
                || error_before != self.operation_error
            {
                cx.notify();
            }
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        if self.selecting {
            let offset = self.display_offset();
            let cell = self.pos_to_cell(event.position, offset);
            if let Some(selection) = self.tabs[self.active].ui.selection.as_mut() {
                selection.head = cell;
            }
        }
        self.selecting = false;
        // A bare click (no drag) clears the selection.
        if self.tabs[self.active]
            .ui
            .selection
            .is_some_and(|selection| selection.is_empty())
        {
            self.tabs[self.active].ui.selection = None;
        }
        cx.notify();
    }

    pub(super) fn report_mouse_motion(&mut self, event: &MouseMoveEvent) -> bool {
        if event.modifiers.shift || !self.tabs[self.active].accepts_input() {
            return false;
        }
        let mode = match self.active_terminal_mode() {
            Ok(mode) => mode,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                return true;
            }
        };
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        let Some(report) = mouse_motion_report(mode, event.pressed_button) else {
            return true;
        };
        let session_id = self.tabs[self.active].id;
        let (row, column) = self.pos_to_viewport_cell(event.position);
        if self.last_mouse_report_cell == Some((session_id, column, row)) {
            return true;
        }
        let Some(bytes) = encode_mouse_report(mode, report, column, row, &event.modifiers) else {
            return true;
        };
        if self.tabs[self.active].write(&bytes).is_ok() {
            self.last_mouse_report_cell = Some((session_id, column, row));
        }
        true
    }

    pub(super) fn report_mouse_wheel(&mut self, event: &ScrollWheelEvent) -> bool {
        if event.modifiers.shift || !self.tabs[self.active].accepts_input() {
            return false;
        }
        let mode = match self.active_terminal_mode() {
            Ok(mode) => mode,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                return true;
            }
        };
        let mouse_reporting = mode.intersects(TermMode::MOUSE_MODE);
        let alternate_scroll = mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL);
        if !mouse_reporting && !alternate_scroll {
            return false;
        }

        let (x_delta, y_delta) = match event.delta {
            ScrollDelta::Lines(delta) => (delta.x, delta.y),
            ScrollDelta::Pixels(delta) => (
                f32::from(delta.x) / self.line_h,
                f32::from(delta.y) / self.line_h,
            ),
        };
        let mut reports = accumulate_wheel_reports(&mut self.mouse_wheel_y_accum, y_delta, 64, 65);
        reports.extend(accumulate_wheel_reports(
            &mut self.mouse_wheel_x_accum,
            x_delta,
            66,
            67,
        ));
        if reports.is_empty() {
            return true;
        }

        let mut bytes = Vec::with_capacity(reports.len() * 16);
        if mouse_reporting {
            let (row, column) = self.pos_to_viewport_cell(event.position);
            for report in reports {
                if let Some(encoded) =
                    encode_mouse_report(mode, report, column, row, &event.modifiers)
                {
                    bytes.extend(encoded);
                }
            }
        } else {
            let modifiers = Modifiers::default();
            for report in reports {
                let MouseReport::Wheel(button) = report else {
                    continue;
                };
                let final_byte = match button {
                    64 => 'A',
                    65 => 'B',
                    _ => continue,
                };
                bytes.extend(cursor_key_sequence(
                    final_byte,
                    &modifiers,
                    mode.contains(TermMode::APP_CURSOR),
                ));
            }
        }
        if !bytes.is_empty() {
            let _ = self.tabs[self.active].write(&bytes);
        }
        true
    }
}
