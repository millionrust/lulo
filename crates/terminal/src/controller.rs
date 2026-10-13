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
mod restore;
mod sheets;
mod shell_commands;
mod tab_lifecycle;
mod view_state;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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
use crate::session::{
    InitialProgram, PasteError, RedrawSender, Session, SessionControlError, SessionWriteError,
};
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
use rmac_ui::{
    AccessibleTextInput as _, Button, Checkbox, InputEvent, InputState, SearchField, TextField,
};
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
        UseSettingsAsDefault,
        ExportSettings,
        ExportTextAs,
        ExportSelectedTextAs,
        Print,
        PrintSelection,
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
        NewWindowWithSameCommand,
        NewTabWithSameCommand,
        CycleProfile,
        ShowTabBar,
        AllowMouseReporting,
        EnterFullScreen,
        ShowProfiles,
        ResetTerminal,
        HardResetTerminal,
        FillScreen,
        ShowSettings,
        ClearToPreviousMark,
        ClearToPreviousBookmark,
        // Shell ▸ New Command…/New Remote Connection…, Show Inspector,
        // Edit Title: each opens or closes its own small overlay sheet.
        // Committing (Run/Connect/Done) is a plain button click, not a
        // dispatched action — only opening and the Escape-to-cancel path
        // need one.
        NewCommand,
        CancelNewCommand,
        NewRemoteConnection,
        CancelNewRemoteConnection,
        EditTitle,
        CancelEditTitle,
        ShowInspector,
        // View ▸ Split Pane (⌘D) / Close Split Pane (⇧⌘D): two scroll
        // positions of the same session, not a second shell.
        SplitPane,
        CloseSplitPane,
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
        // Edit ▸ Marks (TERM-16/TERM-23).
        AutomaticallyMarkPromptLines,
        MarkLineAndSendReturn,
        SendReturnWithoutMarking,
        // Edit ▸ Bookmarks ▸ <bookmark N> (TERM-15/TERM-16): up to
        // `BOOKMARK_MENU_SLOTS` predefined rows, the same fixed-slot
        // convention `rmac_app_menu::recent` uses for File ▸ Open Recent.
        JumpToBookmark0,
        JumpToBookmark1,
        JumpToBookmark2,
        JumpToBookmark3,
        JumpToBookmark4,
        // Edit ▸ Bookmarks ▸ No Bookmarks: an always-disabled placeholder
        // row, like Clock's NoRecentTimers; never actually dispatched.
        NoBookmarks,
        // View (TERM-23).
        ShowMarks,
        ShowAllTabs,
        ShowAlternativeScreen,
        HideAlternativeScreen,
        // Edit ▸ Find (TERM-23).
        FindSelectAll,
        FindSelectAllInSelection,
        // Shell ▸ Open…/Edit Background Colour (TERM-15).
        OpenShell,
        CancelOpenShell,
        EditBackgroundColour,
        CancelEditBackgroundColour,
        // Application ▸ Quit and Keep Windows (TERM-22).
        QuitAndKeepWindows,
        // Edit ▸ Copy Special ▸ Style for "Copy" Command
        // (TRM-MENU-001..015): a session-wide radio choice of which
        // profile's colours a plain Copy renders its styled clipboard
        // content with.
        // A disabled heading row, like Window ▸ Bookmarks' "No Bookmarks":
        // never actually dispatched.
        CopyStyleHeading,
        CopyStyleDefault,
        CopyStylePlainText,
        CopyStyleBasic,
        CopyStyleClearDark,
        CopyStyleClearLight,
        CopyStyleGrass,
        CopyStyleHomebrew,
        CopyStyleManPage,
        CopyStyleNovel,
        CopyStyleOcean,
        CopyStylePro,
        CopyStyleRedSands,
        CopyStyleSilverAerogel,
        CopyStyleSolidColors,
    ]
);

/// Edit ▸ Bookmarks ▸: how many bookmarked lines the submenu lists before
/// falling back to "No Bookmarks", using the same predefined-action-slot
/// convention as `rmac_app_menu::recent`'s Open Recent rows.
pub(super) const BOOKMARK_MENU_SLOTS: usize = 5;

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
    cx.on_action(|_: &CopyStyleHeading, _| {});
    macro_rules! copy_style_action {
        ($action:ty, $style:expr) => {
            cx.on_action(|_: &$action, cx| {
                profiles::set_copy_style($style);
                publish_copy_style_menu_state(cx);
            });
        };
    }
    copy_style_action!(CopyStyleDefault, profiles::CopyStyle::Default);
    copy_style_action!(CopyStylePlainText, profiles::CopyStyle::PlainText);
    copy_style_action!(
        CopyStyleBasic,
        profiles::CopyStyle::Profile(profile_named("Basic"))
    );
    copy_style_action!(
        CopyStyleClearDark,
        profiles::CopyStyle::Profile(profile_named("Clear Dark"))
    );
    copy_style_action!(
        CopyStyleClearLight,
        profiles::CopyStyle::Profile(profile_named("Clear Light"))
    );
    copy_style_action!(
        CopyStyleGrass,
        profiles::CopyStyle::Profile(profile_named("Grass"))
    );
    copy_style_action!(
        CopyStyleHomebrew,
        profiles::CopyStyle::Profile(profile_named("Homebrew"))
    );
    copy_style_action!(
        CopyStyleManPage,
        profiles::CopyStyle::Profile(profile_named("Man Page"))
    );
    copy_style_action!(
        CopyStyleNovel,
        profiles::CopyStyle::Profile(profile_named("Novel"))
    );
    copy_style_action!(
        CopyStyleOcean,
        profiles::CopyStyle::Profile(profile_named("Ocean"))
    );
    copy_style_action!(
        CopyStylePro,
        profiles::CopyStyle::Profile(profile_named("Pro"))
    );
    copy_style_action!(
        CopyStyleRedSands,
        profiles::CopyStyle::Profile(profile_named("Red Sands"))
    );
    copy_style_action!(
        CopyStyleSilverAerogel,
        profiles::CopyStyle::Profile(profile_named("Silver Aerogel"))
    );
    copy_style_action!(
        CopyStyleSolidColors,
        profiles::CopyStyle::Profile(profile_named("Solid Colors"))
    );
    cx.on_action(|_: &ShowSettings, cx| {
        if cx.windows().is_empty() {
            crate::settings_window::show(cx);
        }
    });
    cx.on_action(|_: &QuitAndKeepWindows, cx| quit_and_keep_windows(cx));
}

/// Application ▸ Quit and Keep Windows (TERM-22): capture every open
/// window's tabs (working directory, what each execs, its profile, and a
/// bounded snapshot of its visible text), save them, then quit. The next
/// plain launch reopens them once (`main::kept_window_launch_arguments`).
fn quit_and_keep_windows(cx: &mut gpui::App) {
    let windows: Vec<crate::session_restore::RestoreWindow> = open_terminal_views()
        .into_iter()
        .filter_map(|view| view.upgrade())
        .map(|view| view.read(cx).capture_for_restore())
        .filter(|window| !window.is_empty())
        .collect();
    // The write happens off the UI thread, and `cx.quit()` only runs once
    // it finishes — quitting first would race a still-running write
    // against the process actually exiting.
    cx.spawn(async move |cx| {
        if let Some(path) = crate::session_restore::kept_windows_path() {
            let _ = cx
                .background_executor()
                .spawn(async move { save_kept_windows(&path, &windows) })
                .await;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn save_kept_windows(
    path: &std::path::Path,
    windows: &[crate::session_restore::RestoreWindow],
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(windows).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}
/// Find-match highlight (macOS yellow).
const FIND_HL: u32 = 0xffd60a;

/// Edit ▸ Copy Special ▸ Style for "Copy" Command: tick whichever choice
/// matches the current global [`profiles::copy_style`], unticking every
/// other one — a plain radio group, republished every render alongside
/// Terminal's other menu checkmarks (`renderer.rs`) since the setting
/// itself is process-wide, not tied to one window.
pub(super) fn publish_copy_style_menu_state(cx: &mut gpui::App) {
    // A heading row, like Edit ▸ Bookmarks' "No Bookmarks": never enabled.
    rmac_ui::set_menu_enabled("terminal::CopyStyleHeading", false, cx);
    let current = profiles::copy_style();
    let choices = [
        ("terminal::CopyStyleDefault", profiles::CopyStyle::Default),
        (
            "terminal::CopyStylePlainText",
            profiles::CopyStyle::PlainText,
        ),
        (
            "terminal::CopyStyleBasic",
            profiles::CopyStyle::Profile(profile_named("Basic")),
        ),
        (
            "terminal::CopyStyleClearDark",
            profiles::CopyStyle::Profile(profile_named("Clear Dark")),
        ),
        (
            "terminal::CopyStyleClearLight",
            profiles::CopyStyle::Profile(profile_named("Clear Light")),
        ),
        (
            "terminal::CopyStyleGrass",
            profiles::CopyStyle::Profile(profile_named("Grass")),
        ),
        (
            "terminal::CopyStyleHomebrew",
            profiles::CopyStyle::Profile(profile_named("Homebrew")),
        ),
        (
            "terminal::CopyStyleManPage",
            profiles::CopyStyle::Profile(profile_named("Man Page")),
        ),
        (
            "terminal::CopyStyleNovel",
            profiles::CopyStyle::Profile(profile_named("Novel")),
        ),
        (
            "terminal::CopyStyleOcean",
            profiles::CopyStyle::Profile(profile_named("Ocean")),
        ),
        (
            "terminal::CopyStylePro",
            profiles::CopyStyle::Profile(profile_named("Pro")),
        ),
        (
            "terminal::CopyStyleRedSands",
            profiles::CopyStyle::Profile(profile_named("Red Sands")),
        ),
        (
            "terminal::CopyStyleSilverAerogel",
            profiles::CopyStyle::Profile(profile_named("Silver Aerogel")),
        ),
        (
            "terminal::CopyStyleSolidColors",
            profiles::CopyStyle::Profile(profile_named("Solid Colors")),
        ),
    ];
    for (action, style) in choices {
        rmac_ui::set_menu_checked(action, style == current, cx);
    }
}

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

/// Shell ▸ New Window with Same Command: a fresh window execing exactly
/// what `exec` names, reusing the same `-e PROGRAM ARGS…` argument
/// convention `main.rs` parses for a launch from the command line.
fn open_window_with_same_command(exec: crate::cli::ExecCommand, cx: &mut gpui::App) {
    let mut arguments = vec!["-e".to_string(), exec.program];
    arguments.extend(exec.args);
    rmac_ui::open_another_window(arguments, cx);
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
    /// Terminal ▸ Settings… ▸ Window ▸ "Show window size in title", read
    /// once at creation like the other Settings-backed fields above.
    title_shows_window_size: bool,
    /// Current phase of the cursor-blink animation; always `true` (visible)
    /// when blink is disabled or the window is not focused. The animation
    /// task that flips this exits the instant focus is lost or blink turns
    /// off — nothing ticks while this window is unfocused.
    blink_visible: bool,
    /// Invalidates an in-flight blink task after a newer one starts, so two
    /// never race if focus is regained before the old one has noticed.
    blink_generation: u64,
    /// When the user last typed. Blinking parks with the cursor shown
    /// `CURSOR_PARK_AFTER` later, so an idle focused window stops
    /// repainting itself twice a second.
    blink_last_input: std::time::Instant,
    /// The blink loop parked for inactivity; the next keystroke restarts it.
    blink_parked: bool,
    persistence_error: Option<SharedString>,
    operation_error: Option<SharedString>,
    pending_close: Option<PendingClose>,
    pending_paste: Option<PendingPaste>,
    /// Shell ▸ New Command… (⇧⌘N).
    pending_new_command: Option<sheets::NewCommandSheet>,
    /// Shell ▸ New Remote Connection… (⇧⌘K).
    pending_remote_connection: Option<sheets::RemoteConnectionSheet>,
    /// Shell ▸ Edit Title (⇧⌘I).
    pending_edit_title: Option<sheets::EditTitleSheet>,
    /// Shell ▸ Show/Hide Inspector (⌘I): a non-modal panel, so it does not
    /// appear in `modal_open()`.
    inspector_open: bool,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<rmac_ui::ContextMenuState>,
    /// Last published AT-SPI text projection of the visible grid, and when.
    a11y_cache: Option<TerminalAccessibilityCache>,
    /// A unique id for this window, used only as Shell ▸ Print…'s
    /// `PrintDocument::window_generation`.
    window_generation: u64,
    /// Edit ▸ Marks ▸ Automatically Mark Prompt Lines, read once at
    /// creation like `option_as_meta`; toggling persists for windows
    /// opened after this one.
    automatically_mark_prompt_lines: bool,
    /// View ▸ Show Marks (TERM-23): a gutter indicator beside the grid for
    /// the marks/bookmarks TERM-16 already tracks. Window-local, like
    /// `show_tab_bar`.
    show_marks: bool,
    /// View ▸ Show All Tabs (TERM-23): an Exposé-style grid of this
    /// window's tabs.
    show_all_tabs: bool,
    /// View ▸ Show/Hide Alternative Screen (TERM-23): while the active
    /// tab's program holds the alternate screen (vim, less, …), a manual
    /// override to look at the primary screen underneath without leaving
    /// the program. Ignored — and always false the next time the mode
    /// actually changes — whenever the active tab is not in alternate-
    /// screen mode, so leaving the program always restores the live view.
    viewing_primary_while_alt_screen: bool,
    /// Shell ▸ Open… (⌘O).
    pending_open_shell: Option<sheets::OpenShellSheet>,
    /// Shell ▸ Edit Background Colour (⌥⌘I).
    pending_background_colour: Option<sheets::BackgroundColourSheet>,
    /// Shell ▸ Edit Background Colour (⌥⌘I): this window's live override
    /// of the active profile's background, read once at creation (like
    /// `font_size`) and published to `profiles::active()` each render.
    background_override: Option<u32>,
}

static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Every open Terminal window's view, weakly held so a closed window
    /// drops out on its own. Application ▸ Quit and Keep Windows reads
    /// this to capture every window before quitting, since one process
    /// hosts every Terminal window (`rmac_ui::boot_app_instance`) and the
    /// menu-bar action fires at the app level, not inside any one view.
    static OPEN_TERMINAL_VIEWS: std::cell::RefCell<Vec<gpui::WeakEntity<TerminalView>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Registers a newly created window's view so Quit and Keep Windows can
/// find it later. Called once from [`TerminalView::new`].
fn register_open_view(view: gpui::WeakEntity<TerminalView>) {
    OPEN_TERMINAL_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|existing| existing.upgrade().is_some());
        views.push(view);
    });
}

/// Every still-open Terminal window's view, for Quit and Keep Windows.
pub(super) fn open_terminal_views() -> Vec<gpui::WeakEntity<TerminalView>> {
    OPEN_TERMINAL_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|existing| existing.upgrade().is_some());
        views.clone()
    })
}
