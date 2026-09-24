//! Terminal controller and platform-input orchestration.
//!
//! The Zed stack: `alacritty_terminal` drives the grid/escape-sequence state,
//! `portable-pty` runs the user's shell, and GPUI renders the cell grid. A
//! background thread reads PTY output and feeds the parser; model changes wake
//! the view, which renders the grid and writes keystrokes back to the PTY.

mod focus;
mod ime_bridge;
mod input;
mod lifecycle;
mod pointer;
mod renderer;
mod responsive_layout;
mod tab_lifecycle;
mod view_state;

#[cfg(test)]
mod tests;

use crate::emulator::{
    grid_dimensions, scrollback_limit_for_tab_count, terminal_config, MIN_COLS, MIN_ROWS,
};
#[cfg(test)]
use crate::emulator::{TermSize, SCROLLBACK_LINES};
use crate::hyperlink::LinkTarget;
use crate::ime::{ImeBuffer, MAX_TEXT_BYTES as MAX_IME_TEXT_BYTES};
#[cfg(test)]
use crate::keyboard::encode_key;
use crate::keyboard::{
    cursor_key_sequence, encode_key_event, uses_platform_text_input, KeyEventKind,
};
use crate::mouse::{
    accumulate_wheel_reports, encode_report as encode_mouse_report,
    motion_report as mouse_motion_report, MouseReport,
};
use crate::paste::{PendingPaste, MAX_BYTES as MAX_PASTE_BYTES};
use crate::profiles::{self, active, load as load_profile, save as save_profile, PROFILES};
#[cfg(test)]
use crate::session::EventProxy;
use crate::session::{PasteError, RedrawSender, Session, SessionControlError, SessionWriteError};
use crate::shell_integration::{CommandRangeKind, PromptDirection};
#[cfg(test)]
use crate::ui_state::MAX_SEARCH_QUERY_BYTES;
use crate::ui_state::{bounded_search_query, Selection};
use alacritty_terminal::grid::{Dimensions, Scroll};
#[cfg(test)]
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use gpui::{
    accesskit, canvas, div, prelude::FluentBuilder as _, px, A11ySubtreeBuilder, AppContext as _,
    ClipboardItem, Context, Div, ElementInputHandler, Entity, FocusHandle, Focusable as _,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, KeyUpEvent,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    ParentElement, Pixels, Point, Render, Role, ScrollDelta, ScrollWheelEvent, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_terminal::accessibility::TerminalAccessibilitySnapshot;
use rmac_ui::{Button, InputState, SearchField};
#[cfg(test)]
use vte::ansi::Processor;
use vte::ansi::{ClearMode, Color, Handler as _, NamedColor};

/// A new Terminal window on macOS 26.2 is 80 × 24 (measured).
const COLS: usize = 80;
const ROWS: usize = 24;
pub(super) const MAX_TABS: usize = 16;
/// The Basic profile's cells measure 7.0 × 14.0 pt on the Mac. JetBrains Mono
/// advances 0.6 em, so 7.0 / 0.6 gives the same 7 pt column.
const FONT_SIZE: f32 = 7.0 / 0.6;
const LINE_H: f32 = 14.0;
/// Pre-measurement fallback for the monospace cell advance. The real advance
/// is measured from `rmac_ui::MONO_FONT` through the window text system; this
/// ratio is used only before a window exists or if the glyph cannot resolve.
const CELL_RATIO_FALLBACK: f32 = 0.6;

/// Measure one monospace cell advance in logical pixels.
pub(super) fn measure_cell_w(window: &Window, font_size: f32) -> f32 {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&gpui::font(rmac_ui::MONO_FONT));
    text_system
        .advance(font_id, px(font_size), 'M')
        .map(|size| f32::from(size.width))
        .unwrap_or(font_size * CELL_RATIO_FALLBACK)
}
/// The shared 32 pt title bar (1 pt base line included).
const TITLE_BAR_HEIGHT: f32 = 32.0;
/// With two or more tabs the Mac adds 36 pt: a 28 pt tab track, a 7 pt gap
/// and a 1 pt base line.
const TAB_BAR_HEIGHT: f32 = 36.0;
/// Grid insets measured on an 80 × 24 window: 80 × 7 + 2 × 10 = 580 wide,
/// 24 × 14 + 7 + 10 = 353 tall below the title bar.
const PAD_X: f32 = 10.0;
const PAD_TOP: f32 = 7.0;
const PAD_BOTTOM: f32 = 10.0;
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
        PreviousPrompt,
        NextPrompt,
        SelectCommand,
        SelectCommandOutput,
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
    TITLE_BAR_HEIGHT + if tab_count > 1 { TAB_BAR_HEIGHT } else { 0.0 } + PAD_TOP
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

/// Debounces the visible-grid accessibility projection so a fast-scrolling
/// command can't turn every redraw into a full re-walk of the grid: idle
/// windows never hit this path (nothing calls `cx.notify()`), but an active
/// one (e.g. `yes`, a build log) can call it far more often than a screen
/// reader needs a fresh text snapshot.
struct TerminalAccessibilityCache {
    tab_id: u64,
    computed_at: std::time::Instant,
    snapshot: TerminalAccessibilitySnapshot,
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
    /// Where the visible window starts inside GPUI's window bounds (the
    /// client frame on Linux); pointer positions are window-space.
    content_origin: (f32, f32),
    focus: FocusHandle,
    native_window_title: String,
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
    /// Privacy-safe destination summary for the link under the pointer.
    hovered_link: Option<SharedString>,
    /// Index into `PROFILES` for the active color scheme.
    profile: usize,
    /// Whether the profile picker dropdown is open.
    picker_open: bool,
    persistence_error: Option<SharedString>,
    operation_error: Option<SharedString>,
    pending_close: Option<PendingClose>,
    pending_paste: Option<PendingPaste>,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<rmac_ui::ContextMenuState>,
    /// Last published AT-SPI text projection of the visible grid, and when.
    a11y_cache: Option<TerminalAccessibilityCache>,
}
