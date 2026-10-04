//! Terminal cursor projection, search synchronization, commands, and grid state.

use super::*;

impl TerminalView {
    pub(super) fn tab_bar_visible(&self) -> bool {
        self.show_tab_bar.unwrap_or(self.tabs.len() > 1)
    }

    pub(super) fn toggle_tab_bar(&mut self, cx: &mut Context<Self>) {
        self.show_tab_bar = Some(!self.tab_bar_visible());
        cx.notify();
    }

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

    pub(super) fn clear_screen(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.clear_screen(ClearMode::All);
        }
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    pub(super) fn clear_scrollback(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.grid_mut().clear_history();
            terminal.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].clear_shell_marks();
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    /// View ▸ Split Pane (⌘D): a second, independently scrolled viewport
    /// onto the SAME session's grid — the Mac's split pane shows two scroll
    /// positions of one session, not a second shell (there is only ever
    /// one PTY per tab here). Starts at the live prompt, like the primary
    /// pane, until the user scrolls either one.
    pub(super) fn toggle_split_pane(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let ui = &mut self.tabs[self.active].ui;
        ui.split_offset = if ui.split_offset.is_some() { None } else { Some(0) };
        cx.notify();
    }

    /// View ▸ Close Split Pane (⇧⌘D).
    pub(super) fn close_split_pane(&mut self, cx: &mut Context<Self>) {
        if self.tabs[self.active].ui.split_offset.take().is_some() {
            cx.notify();
        }
    }

    /// Scroll wheel over the split pane's own viewport: its offset is
    /// independent of the primary pane's `Term::grid().display_offset()`.
    pub(super) fn scroll_split_pane(&mut self, lines: i32, cx: &mut Context<Self>) {
        let Ok(term) = self.tabs[self.active].term.lock() else {
            return;
        };
        let history = term.grid().history_size() as i32;
        drop(term);
        let ui = &mut self.tabs[self.active].ui;
        let Some(offset) = ui.split_offset else {
            return;
        };
        let next = (offset + lines).clamp(0, history);
        if next != offset {
            ui.split_offset = Some(next);
            cx.notify();
        }
    }

    pub(super) fn toggle_option_as_meta(&mut self, cx: &mut Context<Self>) {
        let next = !self.option_as_meta;
        match profiles::save_option_as_meta(next) {
            Ok(()) => {
                self.option_as_meta = next;
                cx.notify();
            }
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub(super) fn hide_find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs[self.active].ui.search_open {
            self.capture_active_search_query(cx);
            self.tabs[self.active].ui.search_open = false;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(super) fn use_selection_for_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection_text().filter(|text| !text.is_empty()) else {
            return;
        };
        let query = bounded_search_query(&selection);
        self.search
            .update(cx, |state, cx| state.set_value(query.clone(), window, cx));
        self.tabs[self.active].ui.search_query = query;
        cx.notify();
    }

    pub(super) fn jump_to_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.tabs[self.active].ui.selection else {
            return;
        };
        let line = selection.anchor.0.min(selection.head.0);
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            let grid = terminal.grid();
            let offset = find::display_offset_for(
                line,
                self.rows,
                grid.history_size(),
                grid.display_offset(),
            );
            terminal.scroll_display(Scroll::Bottom);
            terminal.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
            cx.notify();
        }
    }

    pub(super) fn scroll_view(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.scroll_display(scroll);
            cx.notify();
        }
    }

    /// Shell ▸ Reset (⌥⌘R): the RIS soft reset a wedged program would answer
    /// to — cursor, colors, and modes return to their defaults, but the
    /// screen and scrollback are left alone, as on the Mac.
    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.reset_state();
        }
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    /// Shell ▸ Hard Reset (⌃⌥⌘R): the same soft reset, plus the screen and
    /// scrollback are cleared — the recovery for binary output that has left
    /// the terminal unreadable.
    pub(super) fn hard_reset(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
            terminal.reset_state();
            terminal.clear_screen(ClearMode::All);
            terminal.grid_mut().clear_history();
            terminal.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].clear_shell_marks();
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    /// Terminal ▸ Settings… (⌘,): the profile list and font size window.
    pub(super) fn show_settings(&mut self, cx: &mut Context<Self>) {
        crate::settings_window::show(cx);
    }

    /// Dropping a file onto the window inserts its shell-quoted path, as on
    /// the Mac.
    pub(super) fn drop_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        if self.modal_open() || paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|path| crate::paste::shell_quote(&path.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        match self.tabs[self.active].paste(&text, false) {
            Ok(()) => {}
            Err(PasteError::ReviewRequired) => {
                self.pending_paste = Some(PendingPaste::new(self.tabs[self.active].id, text));
            }
            Err(_) => {}
        }
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

    pub(super) fn mark_current_line(&mut self, bookmark: bool, cx: &mut Context<Self>) {
        if !self.modal_open() && self.tabs[self.active].mark_current_line(bookmark) {
            cx.notify();
        }
    }

    pub(super) fn unmark_current_line(&mut self, cx: &mut Context<Self>) {
        if !self.modal_open() && self.tabs[self.active].unmark_current_line() {
            cx.notify();
        }
    }

    /// Edit ▸ Clear to Previous Mark (⌘L) / Clear to Previous Bookmark
    /// (⌥⌘L).
    pub(super) fn clear_to_previous_mark(&mut self, bookmark_only: bool, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        match self.tabs[self.active].clear_to_previous_mark(bookmark_only) {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub(super) fn navigate_bookmark(&mut self, direction: PromptDirection, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        match self.tabs[self.active].scroll_to_bookmark(direction) {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub(super) fn select_to_mark(
        &mut self,
        direction: PromptDirection,
        bookmark_only: bool,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        match self.tabs[self.active].select_to_mark(direction, bookmark_only) {
            Ok(true) => cx.notify(),
            Ok(false) => {}
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
