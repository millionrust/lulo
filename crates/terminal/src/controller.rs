//! Terminal controller and platform-input orchestration.
//!
//! The Zed stack: `alacritty_terminal` drives the grid/escape-sequence state,
//! `portable-pty` runs the user's shell, and GPUI renders the cell grid. A
//! background thread reads PTY output and feeds the parser; model changes wake
//! the view, which renders the grid and writes keystrokes back to the PTY.

mod pointer;
mod renderer;

use std::ops::Range;
use std::sync::Arc;

use crate::emulator::{
    grid_dimensions, scrollback_limit_for_tab_count, terminal_config, MIN_COLS, MIN_ROWS,
};
#[cfg(test)]
use crate::emulator::{TermSize, SCROLLBACK_LINES};
use crate::ime::{
    byte_range_for_utf16, replace_buffer as replace_ime_buffer, utf16_len, ImeBuffer, ImeEditError,
    MAX_TEXT_BYTES as MAX_IME_TEXT_BYTES,
};
use crate::keyboard::{cursor_key_sequence, encode_key, uses_platform_text_input};
use crate::mouse::{
    accumulate_wheel_reports, encode_report as encode_mouse_report,
    motion_report as mouse_motion_report, MouseReport,
};
use crate::paste::{PendingPaste, MAX_BYTES as MAX_PASTE_BYTES};
use crate::profiles::{self, active, load as load_profile, save as save_profile, PROFILES};
#[cfg(test)]
use crate::session::EventProxy;
use crate::session::{PasteError, RedrawSender, Session, SessionControlError, SessionWriteError};
#[cfg(test)]
use crate::ui_state::MAX_SEARCH_QUERY_BYTES;
use crate::ui_state::{bounded_search_query, Selection};
use alacritty_terminal::grid::{Dimensions, Scroll};
#[cfg(test)]
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use gpui::{
    canvas, div, prelude::FluentBuilder as _, px, AppContext as _, Bounds, ClipboardItem, Context,
    Div, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable as _, FontWeight,
    Hsla, InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection, ParentElement, Pixels,
    Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, UTF16Selection, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{Button, InputState, SearchField};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
#[cfg(test)]
use vte::ansi::Processor;
use vte::ansi::{ClearMode, Color, Handler as _, NamedColor};

const COLS: usize = 100;
const ROWS: usize = 28;
pub(super) const MAX_TABS: usize = 16;
const FONT: &str = "Menlo"; // macOS Terminal's default monospace
const FONT_SIZE: f32 = 13.0;
const LINE_H: f32 = 17.0;
/// Approximate monospace cell advance for `Menlo` at `FONT_SIZE`.
const CELL_W: f32 = FONT_SIZE * 0.6;
const TITLE_BAR_HEIGHT: f32 = 34.0;
const TAB_BAR_HEIGHT: f32 = 32.0;
const BODY_PAD: f32 = 8.0;
/// Pixels from the window left to the first column: 8pt content padding.
const LEFT_PAD: f32 = BODY_PAD;
const FOCUS_IN_REPORT: &[u8] = b"\x1b[I";
const FOCUS_OUT_REPORT: &[u8] = b"\x1b[O";

gpui::actions!(
    terminal,
    [
        Copy,
        Paste,
        Find,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        SelectAll,
        Clear,
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        CycleProfile,
        ShowProfiles
    ]
);
/// Find-match highlight (macOS yellow).
const FIND_HL: u32 = 0xffd60a;

fn terminal_content_top(tab_count: usize) -> f32 {
    TITLE_BAR_HEIGHT + if tab_count > 1 { TAB_BAR_HEIGHT } else { 0.0 } + BODY_PAD
}

fn focus_report(mode: TermMode, focused: bool) -> Option<&'static [u8]> {
    mode.contains(TermMode::FOCUS_IN_OUT).then_some(if focused {
        FOCUS_IN_REPORT
    } else {
        FOCUS_OUT_REPORT
    })
}

struct ImeComposition {
    session_id: u64,
    buffer: ImeBuffer,
}

/// Visual style of a run of cells — runs break when any attribute changes.
#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Hsla,
    bg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingClose {
    Tab { session_id: u64 },
    Window { foreground_sessions: usize },
}

pub(super) struct TerminalView {
    tabs: Vec<Session>,
    redraw: RedrawSender,
    active: usize,
    cols: usize,
    rows: usize,
    font_size: f32,
    line_h: f32,
    cell_w: f32,
    focus: FocusHandle,
    /// Last operating-system activation state observed for this window.
    window_active: bool,
    /// One visible editor is synchronized with the active tab's bounded query.
    search: Entity<InputState>,
    /// Private, bounded marked text bound to one exact terminal session.
    ime: Option<ImeComposition>,
    /// True while the mouse button is held during a drag-select.
    selecting: bool,
    /// Fractional scroll-line accumulator for smooth trackpad scrolling.
    scroll_accum: f32,
    mouse_wheel_x_accum: f32,
    mouse_wheel_y_accum: f32,
    reported_mouse_press: Option<(u64, MouseButton)>,
    last_mouse_report_cell: Option<(u64, usize, usize)>,
    /// Index into `PROFILES` for the active color scheme.
    profile: usize,
    /// Whether the profile picker dropdown is open.
    picker_open: bool,
    persistence_error: Option<SharedString>,
    operation_error: Option<SharedString>,
    pending_close: Option<PendingClose>,
    pending_paste: Option<PendingPaste>,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<Point<Pixels>>,
}

impl TerminalView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (redraw, redraw_rx) = async_channel::bounded(1);
        let scrollback_lines = scrollback_limit_for_tab_count(1);
        let session = Session::spawn(COLS, ROWS, scrollback_lines, redraw.clone())
            .unwrap_or_else(|error| Session::failed(COLS, ROWS, scrollback_lines, error));

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        cx.observe(&search, |this, _, cx| {
            let query = bounded_search_query(&this.search.read(cx).value());
            this.tabs[this.active].ui.search_query = query;
            cx.notify();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, Some("Terminal")),
            KeyBinding::new(rmac_ui::shortcuts::PASTE.keystroke, Paste, Some("Terminal")),
            KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, Find, Some("Terminal")),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN.keystroke,
                ZoomIn,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_IN_ALTERNATE.keystroke,
                ZoomIn,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_OUT.keystroke,
                ZoomOut,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ZOOM_RESET.keystroke,
                ZoomReset,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::SELECT_ALL.keystroke,
                SelectAll,
                Some("Terminal"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::CLEAR.keystroke, Clear, Some("Terminal")),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_TAB.keystroke,
                NewTab,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::CLOSE.keystroke,
                CloseTab,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::NEXT_TAB.keystroke,
                NextTab,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::PREVIOUS_TAB.keystroke,
                PrevTab,
                Some("Terminal"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::CYCLE_PROFILE.keystroke,
                CycleProfile,
                Some("Terminal"),
            ),
        ]);

        let focus = cx.focus_handle();
        window.focus(&focus);
        let window_active = window.is_window_active();
        cx.observe_window_activation(window, |this, window, cx| {
            this.handle_window_activation(window.is_window_active(), cx);
        })
        .detach();
        let (profile, persistence_error) = match load_profile() {
            Ok((profile, legacy_index)) => {
                let migration_error = legacy_index
                    .then(|| save_profile(profile).err())
                    .flatten()
                    .map(|failure| SharedString::from(failure.to_string()));
                (profile, migration_error)
            }
            Err(failure) => (0, Some(SharedString::from(failure.to_string()))),
        };

        // PTY/model events wake this task. The bounded channel coalesces output
        // bursts while leaving the application fully asleep when nothing changes.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while redraw_rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            tabs: vec![session],
            redraw,
            active: 0,
            cols: COLS,
            rows: ROWS,
            font_size: FONT_SIZE,
            line_h: LINE_H,
            cell_w: CELL_W,
            focus,
            window_active,
            search,
            ime: None,
            selecting: false,
            scroll_accum: 0.0,
            mouse_wheel_x_accum: 0.0,
            mouse_wheel_y_accum: 0.0,
            reported_mouse_press: None,
            last_mouse_report_cell: None,
            profile,
            picker_open: false,
            persistence_error,
            operation_error: None,
            pending_close: None,
            pending_paste: None,
            menu_at: None,
        }
    }

    fn set_profile(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < PROFILES.len() {
            self.profile = i;
            self.picker_open = false;
            self.persistence_error = save_profile(i)
                .err()
                .map(|failure| failure.to_string().into());
            cx.notify();
        }
    }

    fn modal_open(&self) -> bool {
        self.pending_close.is_some() || self.pending_paste.is_some()
    }

    fn report_focus_for_session(
        &mut self,
        index: usize,
        focused: bool,
    ) -> Result<(), SessionWriteError> {
        if !self.tabs[index].accepts_input() {
            return Ok(());
        }
        let report = {
            let term = self.tabs[index]
                .term
                .lock()
                .map_err(|_| SessionWriteError::State)?;
            focus_report(*term.mode(), focused)
        };
        let Some(report) = report else {
            return Ok(());
        };
        self.tabs[index].write(report)
    }

    /// Report one truthful focus transition without repainting the ordinary
    /// success path. Returns whether visible failure state changed.
    fn report_active_focus(&mut self, focused: bool) -> bool {
        let was_live = self.tabs[self.active].accepts_input();
        let result = self.report_focus_for_session(self.active, focused);
        if matches!(result, Err(SessionWriteError::State)) {
            self.operation_error = Some(SessionWriteError::State.to_string().into());
        }
        was_live != self.tabs[self.active].accepts_input()
            || matches!(result, Err(SessionWriteError::State))
    }

    fn handle_window_activation(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.window_active == active {
            return;
        }
        self.window_active = active;
        if self.report_active_focus(active) {
            cx.notify();
        }
    }

    fn reject_ime(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.ime = None;
        self.operation_error = Some(message.into());
        cx.notify();
    }

    fn commit_text_input(&mut self, session_id: u64, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            cx.notify();
            return;
        }
        if text.len() > MAX_IME_TEXT_BYTES {
            self.reject_ime(
                "Text input exceeds Terminal's 16 KiB composition safety limit; nothing was sent.",
                cx,
            );
            return;
        }
        if text.chars().any(char::is_control) {
            self.reject_ime(
                "Terminal refused non-text control data from the input method.",
                cx,
            );
            return;
        }
        if self.modal_open() {
            self.reject_ime(
                "Terminal did not send text while a confirmation was open.",
                cx,
            );
            return;
        }
        if self.tabs[self.active].id != session_id {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        if let Err(error) = self.active_terminal_mode() {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
            return;
        }

        match self.tabs[self.active].write(text.as_bytes()) {
            Ok(()) => {
                if let Ok(mut term) = self.tabs[self.active].term.lock() {
                    term.scroll_display(Scroll::Bottom);
                }
                self.tabs[self.active].ui.selection = None;
            }
            Err(SessionWriteError::State) => {
                self.operation_error = Some(SessionWriteError::State.to_string().into());
            }
            Err(SessionWriteError::Exited | SessionWriteError::Write) => {}
        }
        cx.notify();
    }

    fn active_cursor_viewport_cell(&self) -> Option<(usize, usize)> {
        let term = self.tabs[self.active].term.lock().ok()?;
        let grid = term.grid();
        let cursor = grid.cursor.point;
        let row = (cursor.line.0 + grid.display_offset() as i32)
            .clamp(0, self.rows.saturating_sub(1) as i32) as usize;
        let column = cursor.column.0.min(self.cols.saturating_sub(1));
        Some((row, column))
    }

    fn capture_active_search_query(&mut self, cx: &Context<Self>) {
        let query = bounded_search_query(&self.search.read(cx).value());
        self.tabs[self.active].ui.search_query = query;
    }

    fn sync_search_editor_to_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.tabs[self.active].ui.search_query.clone();
        self.search
            .update(cx, |state, cx| state.set_value(query, window, cx));
        if self.tabs[self.active].ui.search_open {
            let search_focus = self.search.read(cx).focus_handle(cx);
            window.focus(&search_focus);
        } else {
            window.focus(&self.focus);
        }
    }

    fn apply_scrollback_limit(&self, limit: usize) -> Result<(), SessionWriteError> {
        // Acquire every authority before mutating any, so one poisoned session
        // cannot leave a partially applied cross-tab budget.
        let mut terms = Vec::with_capacity(self.tabs.len());
        for session in &self.tabs {
            terms.push(session.term.lock().map_err(|_| SessionWriteError::State)?);
        }
        for term in &mut terms {
            // `set_options` updates the primary history even while the
            // alternate screen is active, while preserving the alternate
            // grid's zero-history contract. Updating `grid_mut()` directly
            // would target the wrong grid.
            term.set_options(terminal_config(limit));
        }
        Ok(())
    }

    fn rebalance_scrollback(&self) -> Result<(), SessionWriteError> {
        self.apply_scrollback_limit(scrollback_limit_for_tab_count(self.tabs.len()))
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if self.tabs.len() >= MAX_TABS {
            self.operation_error =
                Some(format!("Terminal supports up to {MAX_TABS} tabs in one window.").into());
            cx.notify();
            return;
        }
        let next_tab_count = self.tabs.len() + 1;
        let scrollback_lines = scrollback_limit_for_tab_count(next_tab_count);
        if let Err(error) = self.apply_scrollback_limit(scrollback_lines) {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if self.window_active {
            let _ = self.report_active_focus(false);
        }
        self.capture_active_search_query(cx);
        let (c, r) = (self.cols.max(MIN_COLS), self.rows.max(MIN_ROWS));
        self.tabs.push(
            Session::spawn(c, r, scrollback_lines, self.redraw.clone())
                .unwrap_or_else(|error| Session::failed(c, r, scrollback_lines, error)),
        );
        self.active = self.tabs.len() - 1;
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active {
            let _ = self.report_active_focus(true);
        }
        cx.notify();
    }

    fn request_close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_paste.take().is_some() {
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        if self.pending_close.is_some() || index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.request_close_window(window, cx);
            return;
        }
        let session_id = self.tabs[index].id;
        if self.tabs[index].has_foreground_job() {
            self.pending_close = Some(PendingClose::Tab { session_id });
            self.capture_active_search_query(cx);
            self.tabs[self.active].ui.search_open = false;
            self.picker_open = false;
            self.menu_at = None;
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        if let Err(error) = self.tabs[index].terminate() {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.remove_tab(session_id, window, cx);
    }

    fn remove_tab(&mut self, session_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|session| session.id == session_id)
        else {
            return;
        };
        if self.tabs.len() <= 1 {
            return;
        }
        let previous_active_id = self.tabs[self.active].id;
        self.capture_active_search_query(cx);
        self.tabs.remove(index);
        if self.active > index {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active && self.tabs[self.active].id != previous_active_id {
            let _ = self.report_active_focus(true);
        }
        if let Err(error) = self.rebalance_scrollback() {
            self.operation_error = Some(error.to_string().into());
        }
        cx.notify();
    }

    fn request_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_paste.take().is_some() {
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        if self.pending_close.is_some() {
            return;
        }
        let foreground_sessions = self
            .tabs
            .iter()
            .filter(|session| session.has_foreground_job())
            .count();
        if foreground_sessions == 0 {
            if self.terminate_all().is_err() {
                self.operation_error =
                    Some("Terminal could not terminate every shell safely.".into());
                cx.notify();
                return;
            }
            window.remove_window();
            return;
        }
        self.pending_close = Some(PendingClose::Window {
            foreground_sessions,
        });
        self.capture_active_search_query(cx);
        self.tabs[self.active].ui.search_open = false;
        self.picker_open = false;
        self.menu_at = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn terminate_all(&mut self) -> Result<(), SessionControlError> {
        let mut failed = false;
        for session in &mut self.tabs {
            failed |= session.terminate().is_err();
        }
        if failed {
            Err(SessionControlError)
        } else {
            Ok(())
        }
    }

    fn cancel_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_close = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn cancel_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_paste = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else {
            return;
        };
        match pending {
            PendingClose::Tab { session_id } => {
                let result = self
                    .tabs
                    .iter_mut()
                    .find(|session| session.id == session_id)
                    .map(Session::terminate)
                    .unwrap_or(Ok(()));
                if let Err(error) = result {
                    self.operation_error = Some(error.to_string().into());
                    window.focus(&self.focus);
                    cx.notify();
                    return;
                }
                self.remove_tab(session_id, window, cx);
                window.focus(&self.focus);
            }
            PendingClose::Window { .. } => {
                if self.terminate_all().is_err() {
                    self.operation_error =
                        Some("Terminal could not terminate every shell safely.".into());
                    window.focus(&self.focus);
                    cx.notify();
                    return;
                }
                window.remove_window();
            }
        }
    }

    fn select_tab(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() || i >= self.tabs.len() || i == self.active {
            return;
        }
        if self.window_active {
            let _ = self.report_active_focus(false);
        }
        self.capture_active_search_query(cx);
        self.active = i;
        self.reset_pointer_routing();
        self.sync_search_editor_to_active(window, cx);
        if self.window_active {
            let _ = self.report_active_focus(true);
        }
        cx.notify();
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let next = (self.active + 1) % self.tabs.len();
            self.select_tab(next, window, cx);
        }
    }

    fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let prev = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.select_tab(prev, window, cx);
        }
    }

    /// Clear the screen and scrollback (⌘K).
    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Ok(mut t) = self.tabs[self.active].term.lock() {
            t.clear_screen(ClearMode::All);
            t.grid_mut().clear_history();
            t.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].ui.selection = None;
        cx.notify();
    }

    /// Select the entire buffer (scrollback history + visible screen).
    fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let hist = self.tabs[self.active]
            .term
            .lock()
            .ok()
            .map(|t| t.grid().history_size() as i32)
            .unwrap_or(0);
        self.tabs[self.active].ui.selection = Some(Selection {
            anchor: (-hist, 0),
            head: (self.rows as i32 - 1, self.cols.saturating_sub(1)),
        });
        cx.notify();
    }

    /// Set the font size (clamped) and re-fit the grid to the window next frame.
    fn set_font(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = size.clamp(8.0, 32.0);
        self.line_h = self.font_size * (LINE_H / FONT_SIZE);
        self.cell_w = self.font_size * 0.6;
        cx.notify();
    }

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.capture_active_search_query(cx);
        self.tabs[self.active].ui.search_open = !self.tabs[self.active].ui.search_open;
        if self.tabs[self.active].ui.search_open {
            let h = self.search.read(cx).focus_handle(cx);
            window.focus(&h);
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Recompute the grid from the window size and propagate to the terminal + PTY.
    fn resize_to(&mut self, window: &Window) {
        let vp = window.viewport_size();
        let w = f32::from(vp.width);
        let h = f32::from(vp.height);
        let size = grid_dimensions(w, h, self.cell_w, self.line_h);
        let _ = self.tabs[self.active].resize(size);
        self.cols = self.tabs[self.active].accepted_size.cols;
        self.rows = self.tabs[self.active].accepted_size.lines;
    }

    fn on_key(&mut self, ev: &KeyDownEvent) -> Result<(), SessionWriteError> {
        let bytes = {
            let term = self.tabs[self.active]
                .term
                .lock()
                .map_err(|_| SessionWriteError::State)?;
            encode_key(&ev.keystroke, *term.mode())
        };
        if bytes.is_empty() {
            return Ok(());
        }
        self.tabs[self.active].write(&bytes)?;
        // Only accepted input jumps to the live prompt and clears the visual
        // selection. A failed writer must not consume local UI state.
        if let Ok(mut term) = self.tabs[self.active].term.lock() {
            term.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].ui.selection = None;
        Ok(())
    }

    /// Copy the current selection to the system clipboard.
    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// Paste clipboard text using the active program's exact bracketed-paste
    /// mode. Unprotected multiline content pauses for private-safe review.
    fn request_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if text.len() > MAX_PASTE_BYTES {
            self.operation_error =
                Some("Paste exceeds Terminal's 1 MiB input safety limit.".into());
            cx.notify();
            return;
        }
        match self.tabs[self.active].paste(&text, false) {
            Ok(()) => {}
            Err(PasteError::ReviewRequired) => {
                self.pending_paste = Some(PendingPaste::new(self.tabs[self.active].id, text));
                self.capture_active_search_query(cx);
                self.tabs[self.active].ui.search_open = false;
                self.picker_open = false;
                self.menu_at = None;
                window.focus(&self.focus);
            }
            Err(error) => {
                if !matches!(
                    error,
                    PasteError::Session(SessionWriteError::Exited | SessionWriteError::Write)
                ) {
                    self.operation_error = Some(error.to_string().into());
                }
            }
        }
        cx.notify();
    }

    fn confirm_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_paste.take() else {
            return;
        };
        let Some(index) = self
            .tabs
            .iter()
            .position(|session| session.id == pending.session_id)
        else {
            self.operation_error = Some("The terminal session changed; nothing was pasted.".into());
            window.focus(&self.focus);
            cx.notify();
            return;
        };
        if index != self.active {
            self.operation_error = Some("The active terminal changed; nothing was pasted.".into());
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        if let Err(error) = self.tabs[index].paste(&pending.text, true) {
            if !matches!(
                error,
                PasteError::Session(SessionWriteError::Exited | SessionWriteError::Write)
            ) {
                self.operation_error = Some(error.to_string().into());
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    /// Extract the selected cells as text. Hard line breaks become `\n`, but
    /// soft-wrapped rows (the last cell carries alacritty's `WRAPLINE` flag) are
    /// joined without a newline so a wrapped long line copies as a single line.
    fn selection_text(&self) -> Option<String> {
        let selection = self.tabs[self.active].ui.selection?;
        let term = self.tabs[self.active].term.lock().ok()?;
        Some(selection.text(&term, self.rows, self.cols))
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let Some(composition) = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
        else {
            if range_utf16.is_empty() && range_utf16.start == 0 {
                *adjusted_range = Some(0..0);
                return Some(String::new());
            }
            return None;
        };
        let bytes = byte_range_for_utf16(&composition.buffer.text, range_utf16.clone())?;
        *adjusted_range = Some(range_utf16);
        Some(composition.buffer.text[bytes].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.modal_open() || !self.tabs[self.active].accepts_input() {
            return None;
        }
        let range = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
            .map_or(0..0, |composition| {
                composition.buffer.selection_utf16.clone()
            });
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let composition = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)?;
        Some(0..utf16_len(&composition.buffer.text))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(composition) = self.ime.take() else {
            return;
        };
        self.commit_text_input(composition.session_id, &composition.buffer.text, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composition = self.ime.take();
        let session_id = composition
            .as_ref()
            .map_or(self.tabs[self.active].id, |composition| {
                composition.session_id
            });
        if session_id != self.tabs[self.active].id {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        if let Some(range) = range_utf16 {
            let valid = if let Some(composition) = composition.as_ref() {
                byte_range_for_utf16(&composition.buffer.text, range).is_some()
            } else {
                range.is_empty() && range.start == 0
            };
            if !valid {
                self.reject_ime(
                    "Terminal refused an invalid text-composition replacement.",
                    cx,
                );
                return;
            }
        }
        self.commit_text_input(session_id, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            self.reject_ime(
                "Terminal did not begin text composition while a confirmation was open.",
                cx,
            );
            return;
        }
        if !self.tabs[self.active].accepts_input() {
            self.ime = None;
            cx.notify();
            return;
        }
        if new_text.chars().any(char::is_control) {
            self.reject_ime(
                "Terminal refused non-text control data from the input method.",
                cx,
            );
            return;
        }

        let active_id = self.tabs[self.active].id;
        if self
            .ime
            .as_ref()
            .is_some_and(|composition| composition.session_id != active_id)
        {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        let current = self.ime.as_ref().map(|composition| &composition.buffer);
        match replace_ime_buffer(current, range_utf16, new_text, new_selected_range_utf16) {
            Ok(buffer) if buffer.text.is_empty() => {
                self.ime = None;
                cx.notify();
            }
            Ok(buffer) => {
                let term = Arc::clone(&self.tabs[self.active].term);
                let Ok(mut term) = term.lock() else {
                    self.reject_ime(
                        "Terminal state is unavailable; composed text was not accepted.",
                        cx,
                    );
                    return;
                };
                term.scroll_display(Scroll::Bottom);
                drop(term);
                self.ime = Some(ImeComposition {
                    session_id: active_id,
                    buffer,
                });
                cx.notify();
            }
            Err(ImeEditError::TooLarge) => self.reject_ime(
                "Text input exceeds Terminal's 16 KiB composition safety limit; nothing was sent.",
                cx,
            ),
            Err(ImeEditError::InvalidRange) => self.reject_ime(
                "Terminal refused an invalid text-composition replacement.",
                cx,
            ),
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, column) = self.active_cursor_viewport_cell()?;
        let (prefix_cells, range_cells) = if let Some(composition) = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
        {
            let prefix_bytes =
                byte_range_for_utf16(&composition.buffer.text, 0..range_utf16.start)?;
            let range_bytes = byte_range_for_utf16(&composition.buffer.text, range_utf16.clone())?;
            (
                UnicodeWidthStr::width(&composition.buffer.text[prefix_bytes]),
                UnicodeWidthStr::width(&composition.buffer.text[range_bytes]).max(1),
            )
        } else if range_utf16.is_empty() && range_utf16.start == 0 {
            (0, 1)
        } else {
            return None;
        };

        let linear_cell = column.saturating_add(prefix_cells);
        let candidate_row = row
            .saturating_add(linear_cell / self.cols.max(1))
            .min(self.rows.saturating_sub(1));
        let candidate_column = linear_cell % self.cols.max(1);
        let available_columns = self.cols.saturating_sub(candidate_column).max(1);
        Some(Bounds::new(
            gpui::point(
                element_bounds.left() + px(BODY_PAD + candidate_column as f32 * self.cell_w),
                element_bounds.top() + px(BODY_PAD + candidate_row as f32 * self.line_h),
            ),
            gpui::size(
                px(range_cells.min(available_columns) as f32 * self.cell_w),
                px(self.line_h),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let composition = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)?;
        let (cursor_row, cursor_column) = self.active_cursor_viewport_cell()?;
        let row = (((f32::from(point.y) - self.terminal_content_top()) / self.line_h).floor()
            as i32)
            .clamp(0, self.rows.saturating_sub(1) as i32) as usize;
        let column = (((f32::from(point.x) - LEFT_PAD) / self.cell_w).floor() as i32)
            .clamp(0, self.cols.saturating_sub(1) as i32) as usize;
        let cursor_linear = cursor_row
            .saturating_mul(self.cols)
            .saturating_add(cursor_column);
        let target_linear = row.saturating_mul(self.cols).saturating_add(column);
        let target_cells = target_linear.saturating_sub(cursor_linear);

        let mut cells = 0;
        let mut utf16_offset = 0;
        for character in composition.buffer.text.chars() {
            if cells >= target_cells {
                break;
            }
            cells += UnicodeWidthChar::width(character).unwrap_or(0);
            utf16_offset += character.len_utf16();
        }
        Some(utf16_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_resources_have_explicit_bounds() {
        assert_eq!(MAX_SEARCH_QUERY_BYTES, 4096);
        assert_eq!(FOCUS_IN_REPORT.len(), 3);
        assert_eq!(FOCUS_OUT_REPORT.len(), 3);
        assert_eq!(MAX_TABS, 16);
        assert_eq!(terminal_content_top(1), 42.0);
        assert_eq!(terminal_content_top(2), 74.0);
    }

    #[test]
    fn focus_reports_follow_the_parsed_xterm_mode() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();

        assert_eq!(focus_report(*term.mode(), true), None);
        assert_eq!(focus_report(*term.mode(), false), None);

        parser.advance(&mut term, b"\x1b[?1004h");
        assert!(term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert_eq!(focus_report(*term.mode(), true), Some(FOCUS_IN_REPORT));
        assert_eq!(focus_report(*term.mode(), false), Some(FOCUS_OUT_REPORT));

        parser.advance(&mut term, b"\x1b[?1004l");
        assert!(!term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert_eq!(focus_report(*term.mode(), true), None);
    }

    #[test]
    fn mouse_modes_follow_parsed_xterm_state() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[?1002;1006h");
        let mode = *term.mode();
        assert!(mode.contains(TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE));
        assert!(!mode.contains(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION));

        parser.advance(&mut term, b"\x1b[?1002;1006l");
        assert!(!term
            .mode()
            .intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE));
    }

    #[test]
    fn bracketed_paste_mode_follows_parsed_xterm_state() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(SCROLLBACK_LINES), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[?2004h");
        assert!(term.mode().contains(TermMode::BRACKETED_PASTE));
        parser.advance(&mut term, b"\x1b[?2004l");
        assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
    }

    fn test_keystroke(
        key: &str,
        key_char: Option<&str>,
        modifiers: gpui::Modifiers,
    ) -> gpui::Keystroke {
        gpui::Keystroke {
            key: key.into(),
            key_char: key_char.map(str::to_owned),
            modifiers,
        }
    }

    #[test]
    fn cursor_keys_follow_the_parsed_application_mode() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(SCROLLBACK_LINES), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        let up = test_keystroke("up", None, gpui::Modifiers::default());
        let home = test_keystroke("home", None, gpui::Modifiers::default());

        assert_eq!(encode_key(&up, *term.mode()), b"\x1b[A");
        assert_eq!(encode_key(&home, *term.mode()), b"\x1b[H");

        parser.advance(&mut term, b"\x1b[?1h");
        assert!(term.mode().contains(TermMode::APP_CURSOR));
        assert_eq!(encode_key(&up, *term.mode()), b"\x1bOA");
        assert_eq!(encode_key(&home, *term.mode()), b"\x1bOH");

        parser.advance(&mut term, b"\x1b[?1l");
        assert!(!term.mode().contains(TermMode::APP_CURSOR));
        assert_eq!(encode_key(&up, *term.mode()), b"\x1b[A");
    }

    #[test]
    fn enhanced_keyboard_protocol_is_not_partially_advertised() {
        assert!(!terminal_config(SCROLLBACK_LINES).kitty_keyboard);
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(SCROLLBACK_LINES), &size, EventProxy);
        let mut parser: Processor = Processor::new();

        parser.advance(&mut term, b"\x1b[>1u");
        assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
    }
}
