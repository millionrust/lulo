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
use crate::find::{self, FindMatch, FindStatus};
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
use crate::settings::{self, CursorStyle};
use crate::shell_integration::{CommandRangeKind, PromptDirection};
#[cfg(test)]
use crate::ui_state::MAX_SEARCH_QUERY_BYTES;
use crate::ui_state::{bounded_search_query, Selection};
use alacritty_terminal::grid::{Dimensions, Scroll};
#[cfg(test)]
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use gpui::{
    canvas, div, prelude::FluentBuilder as _, px, A11ySubtreeBuilder, AccessibleAction,
    AppContext as _, ClipboardItem, Context, Div, ElementInputHandler, Entity, ExternalPaths,
    FocusHandle, Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, NavigationDirection, ParentElement, Pixels, Point, Render, Role, ScrollDelta,
    ScrollWheelEvent, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_terminal::accessibility::TerminalAccessibilitySnapshot;
use rmac_ui::{AccessibleTextInput as _, Button, InputEvent, InputState, SearchField};
#[cfg(test)]
use vte::ansi::Processor;
use vte::ansi::{ClearMode, Color, Handler as _, NamedColor};

pub(super) const MAX_TABS: usize = 16;
/// The Basic profile's cells measure 7.0 × 14.0 pt on the Mac. JetBrains Mono
/// advances 0.6 em, so 7.0 / 0.6 gives the same 7 pt column.
pub(crate) const FONT_SIZE: f32 = 7.0 / 0.6;
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
        CopyPlainText,
        CopyStylePlainText,
        CopyWithoutBackgroundColour,
        OpenManPageForSelection,
        SearchManPageIndexForSelection,
        Paste,
        PasteSelection,
        PasteEscapedText,
        PasteEscapedSelection,
        Find,
        FindNext,
        FindPrevious,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        SelectAll,
        Clear,
        ClearScreen,
        ClearScrollback,
        ToggleOptionAsMeta,
        HideFindBar,
        UseSelectionForFind,
        JumpToSelection,
        ScrollToTop,
        ScrollToBottom,
        PageUp,
        PageDown,
        LineUp,
        LineDown,
        PreviousPrompt,
        NextPrompt,
        Mark,
        MarkAsBookmark,
        Unmark,
        PreviousBookmark,
        NextBookmark,
        SelectToPreviousMark,
        SelectToNextMark,
        SelectToPreviousBookmark,
        SelectToNextBookmark,
        SelectCommand,
        SelectCommandOutput,
        CloseTab,
        CloseWindow,
        CloseAll,
        NextTab,
        PrevTab,
        CycleProfile,
        ShowTabBar,
        AllowMouseReporting,
        EnterFullScreen,
        ShowProfiles,
        ResetTerminal,
        HardResetTerminal,
        ShowSettings,
        // Shell ▸ New Window ▸ <profile>: ⌘N and the plain "Basic" row open
        // the same default profile under two distinct actions, matching the
        // Mac's own duplicate rows.
        WindowBasicDefault,
        WindowBasic,
        WindowClearDark,
        WindowClearLight,
        WindowGrass,
        WindowHomebrew,
        WindowManPage,
        WindowNovel,
        WindowOcean,
        WindowPro,
        WindowRedSands,
        WindowSilverAerogel,
        WindowSolidColors,
        // Shell ▸ New Tab ▸ <profile>: same shape as the window submenu.
        TabBasicDefault,
        TabBasic,
        TabClearDark,
        TabClearLight,
        TabGrass,
        TabHomebrew,
        TabManPage,
        TabNovel,
        TabOcean,
        TabPro,
        TabRedSands,
        TabSilverAerogel,
        TabSolidColors,
    ]
);

fn open_windowless_profile(profile: Option<usize>, cx: &mut gpui::App) {
    if cx.windows().is_empty() {
        let arguments = profile
            .map(|index| vec![format!("--profile={index}")])
            .unwrap_or_default();
        rmac_ui::open_another_window(arguments, cx);
    }
}

pub(crate) fn register_windowless_actions(cx: &mut gpui::App) {
    macro_rules! open_profile {
        ($action:ty, $profile:expr) => {
            cx.on_action(|_: &$action, cx| open_windowless_profile($profile, cx));
        };
    }
    open_profile!(WindowBasicDefault, None);
    open_profile!(WindowBasic, Some(profile_named("Basic")));
    open_profile!(WindowClearDark, Some(profile_named("Clear Dark")));
    open_profile!(WindowClearLight, Some(profile_named("Clear Light")));
    open_profile!(WindowGrass, Some(profile_named("Grass")));
    open_profile!(WindowHomebrew, Some(profile_named("Homebrew")));
    open_profile!(WindowManPage, Some(profile_named("Man Page")));
    open_profile!(WindowNovel, Some(profile_named("Novel")));
    open_profile!(WindowOcean, Some(profile_named("Ocean")));
    open_profile!(WindowPro, Some(profile_named("Pro")));
    open_profile!(WindowRedSands, Some(profile_named("Red Sands")));
    open_profile!(WindowSilverAerogel, Some(profile_named("Silver Aerogel")));
    open_profile!(WindowSolidColors, Some(profile_named("Solid Colors")));
    open_profile!(TabBasicDefault, None);
    open_profile!(TabBasic, Some(profile_named("Basic")));
    open_profile!(TabClearDark, Some(profile_named("Clear Dark")));
    open_profile!(TabClearLight, Some(profile_named("Clear Light")));
    open_profile!(TabGrass, Some(profile_named("Grass")));
    open_profile!(TabHomebrew, Some(profile_named("Homebrew")));
    open_profile!(TabManPage, Some(profile_named("Man Page")));
    open_profile!(TabNovel, Some(profile_named("Novel")));
    open_profile!(TabOcean, Some(profile_named("Ocean")));
    open_profile!(TabPro, Some(profile_named("Pro")));
    open_profile!(TabRedSands, Some(profile_named("Red Sands")));
    open_profile!(TabSilverAerogel, Some(profile_named("Silver Aerogel")));
    open_profile!(TabSolidColors, Some(profile_named("Solid Colors")));
    cx.on_action(|_: &ShowSettings, cx| {
        if cx.windows().is_empty() {
            crate::settings_window::show(cx);
        }
    });
}
/// Find-match highlight (macOS yellow).
const FIND_HL: u32 = 0xffd60a;

/// The index of the one built-in profile named exactly `name`, falling
/// back to the default profile. The Shell ▸ New Window/New Tab submenus
/// below name every built-in profile exactly once, so this always finds a
/// match for them.
fn profile_named(name: &str) -> usize {
    PROFILES
        .iter()
        .position(|profile| profile.name == name)
        .unwrap_or(profiles::DEFAULT_PROFILE)
}

/// Shell ▸ New Window ▸ `<profile>`: a fresh Terminal window pinned to that
/// profile from the start, independent of whichever profile the window
/// that opened it is using.
fn open_window_with_profile(name: &str, cx: &mut gpui::App) {
    rmac_ui::open_another_window(vec![format!("--profile={}", profile_named(name))], cx);
}

fn terminal_content_top(show_tab_bar: bool) -> f32 {
    TITLE_BAR_HEIGHT + if show_tab_bar { TAB_BAR_HEIGHT } else { 0.0 } + PAD_TOP
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
    Window,
}

/// Debounces the visible-grid accessibility projection so a fast-scrolling
/// command can't turn every redraw into a full re-walk of the grid: idle
/// windows never hit this path (nothing calls `cx.notify()`), but an active
/// one (e.g. `yes`, a build log) can call it far more often than a screen
/// reader needs a fresh text snapshot. A redraw served from the cache
/// schedules one trailing redraw, so the last output is always published.
struct TerminalAccessibilityCache {
    tab_id: u64,
    computed_at: std::time::Instant,
    snapshot: TerminalAccessibilitySnapshot,
    refresh_scheduled: bool,
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
    /// Color profile for each tab, in the same order as `tabs`.
    tab_profiles: Vec<usize>,
    /// A menu override for automatic tab-strip visibility (normally shown
    /// when there are multiple tabs).
    show_tab_bar: Option<bool>,
    /// Whether application-requested xterm mouse events may leave this window.
    allow_mouse_reporting: bool,
    /// Whether the profile picker dropdown is open.
    picker_open: bool,
    /// Whether ⌥ sends Meta (an Escape prefix) instead of typing the
    /// platform's composed character. Off by default, as on the Mac.
    option_as_meta: bool,
    /// Terminal ▸ Settings… ▸ Text ▸ Cursor style, read once at window
    /// creation — like `profile`/`font_size`, a later Settings change
    /// applies to the next window, not retroactively to this one.
    cursor_style: CursorStyle,
    /// Terminal ▸ Settings… ▸ Text ▸ Blink cursor, read once at creation.
    cursor_blink_enabled: bool,
    use_bold_fonts: bool,
    bright_bold_text: bool,
    display_ansi_colours: bool,
    /// Current phase of the cursor-blink animation; always `true` (visible)
    /// when blink is disabled or the window is not focused. The animation
    /// task that flips this exits the instant focus is lost or blink turns
    /// off — nothing ticks while this window is unfocused.
    blink_visible: bool,
    /// Invalidates an in-flight blink task after a newer one starts, so two
    /// never race if focus is regained before the old one has noticed.
    blink_generation: u64,
    persistence_error: Option<SharedString>,
    operation_error: Option<SharedString>,
    pending_close: Option<PendingClose>,
    pending_paste: Option<PendingPaste>,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<rmac_ui::ContextMenuState>,
    /// Last published AT-SPI text projection of the visible grid, and when.
    a11y_cache: Option<TerminalAccessibilityCache>,
}
