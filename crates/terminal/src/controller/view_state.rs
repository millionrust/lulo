//! Terminal cursor projection, search synchronization, commands, and grid state.

use super::*;

impl TerminalView {
    pub(super) fn active_cursor_viewport_cell(&self) -> Option<(usize, usize)> {
        let term = self.tabs[self.active].term.lock().ok()?;
        let grid = term.grid();
        let cursor = grid.cursor.point;
        let row = (cursor.line.0 + grid.display_offset() as i32)
            .clamp(0, self.rows.saturating_sub(1) as i32) as usize;
        let column = cursor.column.0.min(self.cols.saturating_sub(1));
        Some((row, column))
    }

    pub(super) fn capture_active_search_query(&mut self, cx: &Context<Self>) {
        let query = bounded_search_query(&self.search.read(cx).value());
        self.tabs[self.active].ui.search_query = query;
    }

    pub(super) fn sync_search_editor_to_active(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.tabs[self.active].ui.search_query.clone();
        self.search
            .update(cx, |state, cx| state.set_value(query, window, cx));
        if self.tabs[self.active].ui.search_open {
            let search_focus = self.search.read(cx).focus_handle(cx);
            window.focus(&search_focus, cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    pub(super) fn cancel_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_paste = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Clear the screen and scrollback (⌘K).
    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.clear_screen(ClearMode::All);
            terminal.grid_mut().clear_history();
            terminal.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].clear_shell_marks();
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    pub(super) fn navigate_prompt(&mut self, direction: PromptDirection, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let menu_was_open = self.menu_at.take().is_some();
        match self.tabs[self.active].scroll_to_prompt(direction) {
            Ok(moved) if moved || menu_was_open => cx.notify(),
            Ok(_) => {}
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub(super) fn select_shell_range(&mut self, kind: CommandRangeKind, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let menu_was_open = self.menu_at.take().is_some();
        match self.tabs[self.active].select_command_range(kind) {
            Ok(selected) if selected || menu_was_open => cx.notify(),
            Ok(_) => {}
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    /// Select the entire buffer (scrollback history + visible screen).
    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let history = self.tabs[self.active]
            .term
            .lock()
            .ok()
            .map(|terminal| terminal.grid().history_size() as i32)
            .unwrap_or(0);
        self.tabs[self.active].ui.selection = Some(Selection {
            anchor: (-history, 0),
            head: (self.rows as i32 - 1, self.cols.saturating_sub(1)),
        });
        cx.notify();
    }

    /// Set the font size (clamped) and re-fit the grid to the window next frame.
    pub(super) fn set_font(&mut self, size: f32, window: &Window, cx: &mut Context<Self>) {
        self.font_size = size.clamp(8.0, 32.0);
        self.line_h = self.font_size * (LINE_H / FONT_SIZE);
        self.cell_w = measure_cell_w(window, self.font_size);
        cx.notify();
    }

    pub(super) fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.capture_active_search_query(cx);
        self.tabs[self.active].ui.search_open = !self.tabs[self.active].ui.search_open;
        if self.tabs[self.active].ui.search_open {
            let search_focus = self.search.read(cx).focus_handle(cx);
            window.focus(&search_focus, cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Recompute the grid from the window size and propagate to the terminal + PTY.
    pub(super) fn resize_to(&mut self, window: &Window) {
        self.cell_w = measure_cell_w(window, self.font_size);
        // The grid fills the window the user sees, not GPUI's bounds, which
        // also hold the client frame's shadow margin on Linux.
        let viewport = rmac_ui::window_content_size(window);
        let insets = rmac_ui::window_content_insets(window);
        self.content_origin = (f32::from(insets.left), f32::from(insets.top));
        let width = f32::from(viewport.width) - 2.0 * PAD_X;
        let height = f32::from(viewport.height) - self.terminal_content_top() - PAD_BOTTOM;
        let size = grid_dimensions(width, height, self.cell_w, self.line_h);
        let _ = self.tabs[self.active].resize(size);
        self.cols = self.tabs[self.active].accepted_size.cols;
        self.rows = self.tabs[self.active].accepted_size.lines;
    }
}
