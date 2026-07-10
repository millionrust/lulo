//! rmac Terminal — a fast, native terminal emulator.
//!
//! The Zed stack: `alacritty_terminal` drives the grid/escape-sequence state,
//! `portable-pty` runs the user's shell, and GPUI renders the cell grid. A
//! background thread reads PTY output and feeds the parser; model changes wake
//! the view, which renders the grid and writes keystrokes back to the PTY.

mod storage;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, ClipboardItem, Context, Div, Entity,
    FocusHandle, Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{Selectable as _, Sizable as _, StyledExt as _};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use vte::ansi::{ClearMode, Color, Handler as _, NamedColor, Processor};

type RedrawSender = async_channel::Sender<()>;

const COLS: usize = 100;
const ROWS: usize = 28;
const FONT: &str = "Menlo"; // macOS Terminal's default monospace
const FONT_SIZE: f32 = 13.0;
const LINE_H: f32 = 17.0;
/// Approximate monospace cell advance for `Menlo` at `FONT_SIZE`.
const CELL_W: f32 = FONT_SIZE * 0.6;
/// Pixels from the window top to the first text row: 34pt title bar + 8pt pad.
const TOP_PAD: f32 = 34.0 + 8.0;
/// Pixels from the window left to the first column: 8pt content padding.
const LEFT_PAD: f32 = 8.0;

/// A macOS Terminal–style color profile: window chrome + 16-color ANSI palette.
#[derive(Clone, Copy)]
struct Profile {
    name: &'static str,
    bg: u32,
    fg: u32,
    cursor: u32,
    selection: u32,
    /// ANSI colors: indices 0-7 normal, 8-15 bright.
    ansi: [u32; 16],
}

/// Classic macOS Terminal.app ANSI palette (Basic profile colors).
const MAC_ANSI: [u32; 16] = [
    0x000000, 0x990000, 0x00a600, 0x999900, 0x0000b2, 0xb200b2, 0x00a6b2, 0xbfbfbf, 0x666666,
    0xe50000, 0x00d900, 0xe5e500, 0x0000ff, 0xe500e5, 0x00e5e5, 0xe5e5e5,
];

/// One Dark ANSI palette (the rmac default look).
const ONE_DARK: [u32; 16] = [
    0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf, 0x5c6370,
    0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
];

/// Built-in profiles mirroring macOS Terminal.app presets. Index 0 is the
/// rmac default (a dark One Dark variant); the rest match Terminal.app.
static PROFILES: &[Profile] = &[
    Profile {
        name: "rmac Dark",
        bg: 0x1e1e1e,
        fg: 0xd4d4d4,
        cursor: 0xd4d4d4,
        selection: 0x2f5d8c,
        ansi: ONE_DARK,
    },
    Profile {
        name: "Basic",
        bg: 0xffffff,
        fg: 0x000000,
        cursor: 0x000000,
        selection: 0xb4d5fe,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Pro",
        bg: 0x000000,
        fg: 0xf2f2f2,
        cursor: 0x4d4d4d,
        selection: 0x414141,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Homebrew",
        bg: 0x000000,
        fg: 0x00ff00,
        cursor: 0x23ff18,
        selection: 0x083905,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Grass",
        bg: 0x13773d,
        fg: 0xfff0a5,
        cursor: 0x8c1543,
        selection: 0x004d00,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Man Page",
        bg: 0xfef49c,
        fg: 0x000000,
        cursor: 0x7f7f7f,
        selection: 0xa3d7ff,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Novel",
        bg: 0xdfdbc3,
        fg: 0x3b2322,
        cursor: 0x73635a,
        selection: 0xa4a390,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Ocean",
        bg: 0x224fbc,
        fg: 0xffffff,
        cursor: 0x7f7f7f,
        selection: 0x216dff,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Red Sands",
        bg: 0x7a251e,
        fg: 0xd7c9a7,
        cursor: 0xffffff,
        selection: 0xa4a390,
        ansi: MAC_ANSI,
    },
];

thread_local! {
    /// The profile in effect for the current render pass, set at the top of
    /// `render()` so the free color functions (`conv`/`named`/`indexed`) resolve
    /// against the active palette without threading state through every call.
    static ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn active() -> &'static Profile {
    let i = ACTIVE.with(|a| a.get());
    PROFILES.get(i).unwrap_or(&PROFILES[0])
}

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

/// Grid geometry handed to the terminal model and the PTY.
#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A selected cell range, in alacritty grid-line coordinates (`Line` values,
/// which are negative for scrollback). Coordinates are `(line, column)`.
#[derive(Clone, Copy)]
struct Selection {
    anchor: (i32, usize),
    head: (i32, usize),
}

impl Selection {
    /// Return `(start, end)` ordered top-to-bottom, left-to-right.
    fn ordered(&self) -> ((i32, usize), (i32, usize)) {
        if (self.anchor.0, self.anchor.1) <= (self.head.0, self.head.1) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Is the cell at `(line, col)` inside the selection?
    fn contains(&self, line: i32, col: usize) -> bool {
        let (s, e) = self.ordered();
        (line, col) >= s && (line, col) <= e
    }
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

#[derive(Clone)]
struct EventProxy;

impl EventListener for EventProxy {}

/// Coalesce any number of PTY/model events into one pending UI repaint.
fn request_redraw(redraw: &RedrawSender) {
    let _ = redraw.try_send(());
}

fn shell_program(configured: Option<String>) -> String {
    configured
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// One terminal tab: its own PTY + parser-fed grid. `master` is `None` for a
/// failed session (PTY/shell couldn't start) — it still renders an error grid.
struct Session {
    term: Arc<Mutex<Term<EventProxy>>>,
    writer: Box<dyn Write + Send>,
    master: Option<Box<dyn MasterPty + Send>>,
}

impl Session {
    /// Start a real shell in a PTY. Returns an error string (rather than
    /// panicking) if the PTY or shell can't be created, so the app stays alive.
    fn spawn(cols: usize, rows: usize, redraw: RedrawSender) -> Result<Session, String> {
        let size = TermSize { cols, lines: rows };
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty failed: {e}"))?;
        let shell = shell_program(std::env::var("SHELL").ok());
        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        let _child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("could not start shell: {e}"))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader failed: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("writer failed: {e}"))?;
        let term = Arc::new(Mutex::new(Term::new(Config::default(), &size, EventProxy)));
        let term_reader = term.clone();
        std::thread::spawn(move || {
            let mut parser: Processor = Processor::new();
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut t) = term_reader.lock() {
                            parser.advance(&mut *t, &buf[..n]);
                        }
                        request_redraw(&redraw);
                    }
                }
            }
            request_redraw(&redraw);
        });
        Ok(Session {
            term,
            writer,
            master: Some(pair.master),
        })
    }

    /// A no-PTY session that just displays an error message in its grid, so a
    /// shell-startup failure degrades gracefully instead of crashing.
    fn failed(cols: usize, rows: usize, msg: &str) -> Session {
        let size = TermSize { cols, lines: rows };
        let term = Arc::new(Mutex::new(Term::new(Config::default(), &size, EventProxy)));
        if let Ok(mut t) = term.lock() {
            let mut parser: Processor = Processor::new();
            let text = format!("\r\n  Terminal unavailable — {msg}\r\n");
            parser.advance(&mut *t, text.as_bytes());
        }
        Session {
            term,
            writer: Box::new(std::io::sink()),
            master: None,
        }
    }
}

struct TerminalView {
    tabs: Vec<Session>,
    redraw: RedrawSender,
    active: usize,
    cols: usize,
    rows: usize,
    font_size: f32,
    line_h: f32,
    cell_w: f32,
    focus: FocusHandle,
    /// Find bar: input + whether it's open.
    search: Entity<InputState>,
    searching: bool,
    /// Active text selection (set while dragging, kept until next click).
    selection: Option<Selection>,
    /// True while the mouse button is held during a drag-select.
    selecting: bool,
    /// Fractional scroll-line accumulator for smooth trackpad scrolling.
    scroll_accum: f32,
    /// Index into `PROFILES` for the active color scheme.
    profile: usize,
    /// Whether the profile picker dropdown is open.
    picker_open: bool,
    persistence_error: Option<SharedString>,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<Point<Pixels>>,
}

/// Path to the persisted stable profile-name file.
fn profile_config_path() -> Result<PathBuf, storage::Failure> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            Path::new("profile.txt"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/rmac-terminal");
    #[cfg(not(target_os = "macos"))]
    let dir = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path.join("rmac-terminal"),
        _ => home.join(".config/rmac-terminal"),
    };
    Ok(dir.join("profile.txt"))
}

fn parse_profile(content: &str) -> Result<(usize, bool), String> {
    let value = content.trim();
    if value.is_empty() {
        return Err("profile preference is empty".into());
    }
    if let Some(index) = PROFILES.iter().position(|profile| profile.name == value) {
        return Ok((index, false));
    }
    if let Ok(index) = value.parse::<usize>() {
        return (index < PROFILES.len())
            .then_some((index, true))
            .ok_or_else(|| format!("legacy profile index {index} is out of range"));
    }
    Err(format!("unknown terminal profile '{value}'"))
}

fn load_profile() -> Result<(usize, bool), storage::Failure> {
    let path = profile_config_path()?;
    match storage::load_optional(&storage::RealStorage, &path)? {
        Some(content) => parse_profile(&content).map_err(|detail| {
            storage::Failure::message(storage::Operation::LoadProfile, &path, detail)
        }),
        None => Ok((0, false)),
    }
}

fn save_profile(index: usize) -> Result<(), storage::Failure> {
    let path = profile_config_path()?;
    let profile = PROFILES.get(index).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::SaveProfile,
            &path,
            format!("profile index {index} is out of range"),
        )
    })?;
    storage::save(&storage::RealStorage, &path, profile.name)
}

impl TerminalView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (redraw, redraw_rx) = async_channel::bounded(1);
        let session = Session::spawn(COLS, ROWS, redraw.clone())
            .unwrap_or_else(|e| Session::failed(COLS, ROWS, &e));

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        cx.bind_keys([
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-f", Find, Some("Terminal")),
            KeyBinding::new("cmd-=", ZoomIn, Some("Terminal")),
            KeyBinding::new("cmd-+", ZoomIn, Some("Terminal")),
            KeyBinding::new("cmd--", ZoomOut, Some("Terminal")),
            KeyBinding::new("cmd-0", ZoomReset, Some("Terminal")),
            KeyBinding::new("cmd-a", SelectAll, Some("Terminal")),
            KeyBinding::new("cmd-k", Clear, Some("Terminal")),
            KeyBinding::new("cmd-t", NewTab, Some("Terminal")),
            KeyBinding::new("cmd-w", CloseTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-p", CycleProfile, Some("Terminal")),
        ]);

        let focus = cx.focus_handle();
        window.focus(&focus);
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
            search,
            searching: false,
            selection: None,
            selecting: false,
            scroll_accum: 0.0,
            profile,
            picker_open: false,
            persistence_error,
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

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        let (c, r) = (self.cols.max(20), self.rows.max(5));
        self.tabs.push(
            Session::spawn(c, r, self.redraw.clone()).unwrap_or_else(|e| Session::failed(c, r, &e)),
        );
        self.active = self.tabs.len() - 1;
        self.selection = None;
        self.cols = 0; // force resize_to() to re-fit the new active session
        cx.notify();
    }

    fn close_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.selection = None;
        self.cols = 0;
        cx.notify();
    }

    fn select_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        self.active = i;
        self.selection = None;
        self.cols = 0;
        cx.notify();
    }

    fn next_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let next = (self.active + 1) % self.tabs.len();
            self.select_tab(next, cx);
        }
    }

    fn prev_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let prev = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.select_tab(prev, cx);
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.tabs.len();
        let active_tab = self.active;
        let mut bar = div()
            .h(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(hsla(0x2a2a2a))
            .border_b_1()
            .border_color(hsla(0x3a3a3a));
        for i in 0..n {
            let is_active = i == active_tab;
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(22.0))
                    .px_2()
                    .rounded(px(5.0))
                    .when(is_active, |el: Div| el.bg(hsla(active().bg)))
                    .child(
                        div()
                            .id(("tabname", i))
                            .text_size(px(12.0))
                            .text_color(hsla(if is_active { 0xffffff } else { 0x9a9a9a }))
                            .child(format!("Terminal {}", i + 1))
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(("tabclose", i))
                            .text_size(px(13.0))
                            .text_color(hsla(0x888888))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.active = i.min(this.tabs.len().saturating_sub(1));
                                this.close_tab(cx);
                            })),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            div()
                .id("newtab")
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(px(15.0))
                .text_color(hsla(0xaaaaaa))
                .child("+")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }

    /// The profile chip in the toolbar — shows the active scheme; click to
    /// open the picker (matching Terminal.app's profile switcher).
    fn profile_chip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = PROFILES[self.profile].name;
        // A gpui-component Button — unlike a raw div, it receives clicks inside
        // the draggable TitleBar (same pattern as other apps' toolbar buttons).
        Button::new("profile-chip")
            .label(format!("{name}  ▼"))
            .ghost()
            .small()
            .selected(self.picker_open)
            .on_click(cx.listener(|this, _, _, cx| {
                this.picker_open = !this.picker_open;
                cx.notify();
            }))
    }

    /// The dropdown list of color profiles.
    fn render_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_idx = self.profile;
        div()
            .absolute()
            .top(px(34.0))
            .right_2()
            .w(px(190.0))
            .bg(hsla(0xffffff))
            .rounded(px(8.0))
            .border_1()
            .border_color(hsla(0xd2d2d2))
            .shadow_lg()
            .py_1()
            .children(PROFILES.iter().enumerate().map(|(i, p)| {
                let is_active = i == active_idx;
                div()
                    .id(("profrow", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(26.0))
                    .px_2()
                    .text_size(px(12.0))
                    .text_color(hsla(0x1d1d1d))
                    .hover(|h| h.bg(hsla(0x4a90e2)).text_color(hsla(0xffffff)))
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(hsla(0xbbbbbb))
                            .bg(hsla(p.bg)),
                    )
                    .child(div().flex_1().child(p.name))
                    .when(is_active, |el: Stateful<Div>| {
                        el.child(div().text_color(hsla(0x4a90e2)).child("✓"))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_profile(i, cx)))
            }))
    }

    /// Clear the screen and scrollback (⌘K).
    fn clear(&mut self, cx: &mut Context<Self>) {
        if let Ok(mut t) = self.tabs[self.active].term.lock() {
            t.clear_screen(ClearMode::All);
            t.grid_mut().clear_history();
            t.scroll_display(Scroll::Bottom);
        }
        self.selection = None;
        cx.notify();
    }

    /// Select the entire buffer (scrollback history + visible screen).
    fn select_all(&mut self, cx: &mut Context<Self>) {
        let hist = self.tabs[self.active]
            .term
            .lock()
            .ok()
            .map(|t| t.grid().history_size() as i32)
            .unwrap_or(0);
        self.selection = Some(Selection {
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
        self.cols = 0; // force resize_to() to recompute on the next render
        cx.notify();
    }

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = !self.searching;
        if self.searching {
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
        let cols = (((w - 16.0) / self.cell_w).floor() as usize).max(20);
        let rows = (((h - 34.0 - 16.0) / self.line_h).floor() as usize).max(5);
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        if let Ok(mut t) = self.tabs[self.active].term.lock() {
            t.resize(TermSize { cols, lines: rows });
        }
        if let Some(master) = self.tabs[self.active].master.as_ref() {
            let _ = master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    /// Current scrollback offset (0 = pinned to the live prompt).
    fn display_offset(&self) -> i32 {
        self.tabs[self.active]
            .term
            .lock()
            .map(|t| t.grid().display_offset() as i32)
            .unwrap_or(0)
    }

    /// Convert a window-space mouse position to a `(grid_line, column)` cell.
    /// `offset` is the scrollback offset so history selections stay anchored
    /// to content rather than to the viewport.
    fn pos_to_cell(&self, pos: Point<Pixels>, offset: i32) -> (i32, usize) {
        let x = f32::from(pos.x);
        let y = f32::from(pos.y);
        let col =
            (((x - LEFT_PAD) / self.cell_w).floor() as i32).clamp(0, self.cols as i32 - 1) as usize;
        let row = (((y - TOP_PAD) / self.line_h).floor() as i32).clamp(0, self.rows as i32 - 1);
        (row - offset, col)
    }

    /// Scroll the viewport by `lines` (positive = into history).
    fn scroll_lines(&mut self, lines: i32) {
        if lines == 0 {
            return;
        }
        if let Ok(mut t) = self.tabs[self.active].term.lock() {
            t.scroll_display(Scroll::Delta(lines));
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        // Let ⌘-shortcuts (copy/paste/…) flow to the action system instead of
        // writing the literal character to the PTY.
        if m.platform {
            return;
        }
        let bytes: Vec<u8> = match ks.key.as_str() {
            "enter" => vec![b'\r'],
            "backspace" => vec![0x7f],
            "tab" => vec![b'\t'],
            "escape" => vec![0x1b],
            "up" => b"\x1b[A".to_vec(),
            "down" => b"\x1b[B".to_vec(),
            "right" => b"\x1b[C".to_vec(),
            "left" => b"\x1b[D".to_vec(),
            _ => {
                if m.control {
                    // Ctrl-letter → control code (Ctrl-C = 0x03, etc.)
                    match ks.key.chars().next() {
                        Some(c) if c.is_ascii_alphabetic() => {
                            vec![(c.to_ascii_uppercase() as u8) & 0x1f]
                        }
                        _ => vec![],
                    }
                } else if let Some(text) = &ks.key_char {
                    text.as_bytes().to_vec()
                } else {
                    vec![]
                }
            }
        };
        if !bytes.is_empty() {
            // Typing jumps the viewport back to the live prompt, like a real terminal.
            if let Ok(mut t) = self.tabs[self.active].term.lock() {
                t.scroll_display(Scroll::Bottom);
            }
            let _ = self.tabs[self.active].writer.write_all(&bytes);
            let _ = self.tabs[self.active].writer.flush();
        }
    }

    /// Copy the current selection to the system clipboard.
    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// Paste clipboard text into the PTY input stream.
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            if let Ok(mut t) = self.tabs[self.active].term.lock() {
                t.scroll_display(Scroll::Bottom);
            }
            let _ = self.tabs[self.active].writer.write_all(text.as_bytes());
            let _ = self.tabs[self.active].writer.flush();
        }
    }

    /// Extract the selected cells as text. Hard line breaks become `\n`, but
    /// soft-wrapped rows (the last cell carries alacritty's `WRAPLINE` flag) are
    /// joined without a newline so a wrapped long line copies as a single line.
    fn selection_text(&self) -> Option<String> {
        let sel = self.selection?;
        let (s, e) = sel.ordered();
        let term = self.tabs[self.active].term.lock().ok()?;
        let grid = term.grid();
        let history = grid.total_lines().saturating_sub(grid.screen_lines()) as i32;
        let last_col = self.cols - 1;

        let mut out = String::new();
        let mut first = true;
        // True when the previous emitted row soft-wrapped into this one.
        let mut prev_wrapped = false;
        for line in s.0..=e.0 {
            if line < -history || line >= self.rows as i32 {
                continue;
            }
            let (c0, c1) = if s.0 == e.0 {
                (s.1, e.1)
            } else if line == s.0 {
                (s.1, last_col)
            } else if line == e.0 {
                (0, e.1)
            } else {
                (0, last_col)
            };
            let row = &grid[Line(line)];
            let mut text = String::new();
            for col in c0..=c1.min(last_col) {
                let ch = row[Column(col)].c;
                text.push(if ch == '\0' { ' ' } else { ch });
            }
            // The row soft-wraps when its final cell is flagged WRAPLINE and the
            // selection reaches that cell, so the next row continues this line.
            let wrapped = c1 >= last_col && row[Column(last_col)].flags.contains(Flags::WRAPLINE);

            if !first && !prev_wrapped {
                out.push('\n');
            }
            // Don't trim a soft-wrapped row: its trailing cells are real content
            // that continues onto the next row.
            if wrapped {
                out.push_str(&text);
            } else {
                out.push_str(text.trim_end());
            }
            prev_wrapped = wrapped;
            first = false;
        }
        Some(out)
    }

    /// Build the visible grid as one styled line per row (style runs), honoring
    /// the scrollback offset, selection highlight, and bold/italic/underline.
    fn render_rows(&self, query: &str) -> Vec<gpui::AnyElement> {
        let Ok(term) = self.tabs[self.active].term.lock() else {
            return Vec::new();
        };
        let grid = term.grid();
        let offset = grid.display_offset() as i32;
        let cursor = grid.cursor.point;
        // Hide the cursor while scrolled back into history.
        let show_cursor = offset == 0;
        let cursor_line = cursor.line.0;
        let cursor_col = cursor.column.0;

        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(self.rows);
        for i in 0..self.rows as i32 {
            let line_idx = i - offset;
            let row = &grid[Line(line_idx)];
            // Columns covered by a find-match in this row (ASCII-approx).
            let matched: Vec<bool> = if query.is_empty() {
                Vec::new()
            } else {
                let text: String = (0..self.cols)
                    .map(|c| {
                        let ch = row[Column(c)].c;
                        if ch == '\0' {
                            ' '
                        } else {
                            ch
                        }
                    })
                    .collect::<String>()
                    .to_lowercase();
                let mut m = vec![false; self.cols];
                let qlen = query.chars().count().max(1);
                let mut start = 0;
                while let Some(pos) = text.get(start..).and_then(|t| t.find(query)) {
                    let s = start + pos;
                    for matched in m.iter_mut().take((s + qlen).min(self.cols)).skip(s) {
                        *matched = true;
                    }
                    start = s + qlen;
                    if start >= text.len() {
                        break;
                    }
                }
                m
            };
            let mut spans: Vec<gpui::AnyElement> = Vec::new();
            let mut run = String::new();
            let mut run_style: Option<Style> = None;

            for col in 0..self.cols {
                let cell = &row[Column(col)];
                let flags = cell.flags;
                let mut fg = conv(cell.fg);
                let mut bg = conv(cell.bg);

                // Dim attribute: fade the foreground.
                if flags.contains(Flags::DIM) {
                    fg.a *= 0.65;
                }
                // Block cursor: invert the cell under the cursor (live view only).
                if show_cursor && line_idx == cursor_line && col == cursor_col {
                    std::mem::swap(&mut fg, &mut bg);
                }
                // Selection highlight overrides the background.
                if let Some(sel) = &self.selection {
                    if !sel.is_empty() && sel.contains(line_idx, col) {
                        bg = hsla(active().selection);
                    }
                }
                // Find-match highlight: yellow with dark text.
                if matched.get(col).copied().unwrap_or(false) {
                    bg = hsla(FIND_HL);
                    fg = hsla(active().bg);
                }

                let style = Style {
                    fg,
                    bg,
                    bold: flags.intersects(Flags::BOLD | Flags::DIM_BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strike: flags.contains(Flags::STRIKEOUT),
                };

                let ch = if cell.c == '\0' { ' ' } else { cell.c };

                match run_style {
                    None => run_style = Some(style),
                    Some(s) if s != style => {
                        spans.push(span(&run, s));
                        run.clear();
                        run_style = Some(style);
                    }
                    _ => {}
                }
                run.push(ch);
            }
            if let Some(s) = run_style {
                if !run.is_empty() {
                    spans.push(span(&run, s));
                }
            }

            rows.push(
                div()
                    .flex()
                    .h(px(self.line_h))
                    .children(spans)
                    .into_any_element(),
            );
        }
        rows
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ACTIVE.with(|a| a.set(self.profile));
        self.resize_to(window);
        let query = if self.searching {
            self.search.read(cx).value().to_lowercase()
        } else {
            String::new()
        };
        let rows = self.render_rows(&query);
        let searching = self.searching;
        let multi = self.tabs.len() > 1;
        let persistence_error = self.persistence_error.clone();
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
                    .text_size(px(13.0))
                    .child(div().flex_1())
                    .child("Terminal")
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
            .when(multi, |el: Div| el.child(self.render_tabs(cx)))
            .child(
                div()
                    .track_focus(&self.focus)
                    .key_context("Terminal")
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                        this.on_key(ev);
                        cx.notify();
                    }))
                    .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
                    .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
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
                    .on_action(cx.listener(|this, _: &NewTab, _, cx| this.new_tab(cx)))
                    .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close_tab(cx)))
                    .on_action(cx.listener(|this, _: &NextTab, _, cx| this.next_tab(cx)))
                    .on_action(cx.listener(|this, _: &PrevTab, _, cx| this.prev_tab(cx)))
                    .on_action(cx.listener(|this, _: &CycleProfile, _, cx| {
                        // ⌘⇧P toggles the profile picker.
                        this.picker_open = !this.picker_open;
                        cx.notify();
                    }))
                    .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                        this.menu_at = None;
                        cx.notify();
                    }))
                    .on_action(
                        cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                            window.remove_window()
                        }),
                    )
                    .on_action(cx.listener(|this, _: &ShowProfiles, _, cx| {
                        // Right-click → Profiles… — a guaranteed mouse path to the
                        // picker (the picker rows are clickable body overlays).
                        this.picker_open = true;
                        cx.notify();
                    }))
                    // Drag to select a cell range.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            if this.picker_open {
                                this.picker_open = false;
                            }
                            let offset = this.display_offset();
                            let cell = this.pos_to_cell(ev.position, offset);
                            this.selection = Some(Selection {
                                anchor: cell,
                                head: cell,
                            });
                            this.selecting = true;
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        if this.selecting {
                            let offset = this.display_offset();
                            let cell = this.pos_to_cell(ev.position, offset);
                            if let Some(sel) = this.selection.as_mut() {
                                sel.head = cell;
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.selecting = false;
                            // A bare click (no drag) clears the selection.
                            if let Some(sel) = this.selection {
                                if sel.is_empty() {
                                    this.selection = None;
                                }
                            }
                            cx.notify();
                        }),
                    )
                    // Scroll wheel / trackpad → walk through scrollback history.
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
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
                    .p_2()
                    .bg(hsla(active().bg))
                    .font_family(FONT)
                    .text_size(px(self.font_size))
                    .v_flex()
                    .children(rows)
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.menu_at = Some(ev.position);
                            cx.notify();
                        }),
                    ),
            )
            .when(searching, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(40.0))
                        .right(px(12.0))
                        .w(px(240.0))
                        .h(px(30.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .rounded(px(7.0))
                        .bg(hsla(0xf0f0f0))
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.search).appearance(false)),
                        )
                        .child(
                            div()
                                .id("find-close")
                                .text_color(hsla(0x666666))
                                .child("×")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.searching = false;
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
            .when_some(self.menu_at, |el: Div, pos| {
                el.child(
                    rmac_ui::ContextMenu::new(pos)
                        .item("Copy", Box::new(Copy))
                        .item("Paste", Box::new(Paste))
                        .item("Select All", Box::new(SelectAll))
                        .separator()
                        .item("Clear", Box::new(Clear))
                        .separator()
                        .item("Profiles…", Box::new(ShowProfiles))
                        .render(),
                )
            })
            .when_some(persistence_error, |terminal, message| {
                terminal.child(
                    div()
                        .id("persistence-error")
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
                        .bg(gpui::rgba(0x7f1d1ddd))
                        .text_size(px(12.0))
                        .text_color(gpui::white())
                        .shadow_lg()
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.persistence_error = None;
                            cx.notify();
                        })),
                )
            })
    }
}

fn span(text: &str, s: Style) -> gpui::AnyElement {
    let mut d = div().text_color(s.fg).bg(s.bg).child(text.to_string());
    if s.bold {
        d = d.font_weight(FontWeight::BOLD);
    }
    if s.italic {
        d = d.italic();
    }
    if s.underline {
        d = d.underline();
    }
    if s.strike {
        d = d.line_through();
    }
    d.into_any_element()
}

fn hsla(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Map a terminal color to RGB.
fn conv(c: Color) -> Hsla {
    let (r, g, b) = match c {
        Color::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        Color::Named(n) => named(n),
        Color::Indexed(i) => indexed(i),
    };
    gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)).into()
}

fn named(n: NamedColor) -> (u8, u8, u8) {
    use NamedColor::*;
    let p = active();
    let ansi = |i: usize| split(p.ansi[i]);
    match n {
        Background => split(p.bg),
        Foreground => split(p.fg),
        Cursor => split(p.cursor),
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
        _ => split(p.fg),
    }
}

fn indexed(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => split(active().ansi[i as usize]),
        16..=231 => {
            let i = i - 16;
            let f = |v: u8| -> u8 {
                if v == 0 {
                    0
                } else {
                    55 + 40 * v
                }
            };
            (f(i / 36), f((i % 36) / 6), f(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
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

fn main() {
    rmac_ui::boot("Terminal", 820.0, 560.0, |window, cx| {
        TerminalView::new(window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraw_requests_coalesce_until_the_ui_consumes_one() {
        let (sender, receiver) = async_channel::bounded(1);

        request_redraw(&sender);
        request_redraw(&sender);
        request_redraw(&sender);

        assert_eq!(receiver.len(), 1);
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn shell_fallback_is_portable_and_ignores_empty_configuration() {
        assert_eq!(shell_program(None), "/bin/sh");
        assert_eq!(shell_program(Some("  ".to_string())), "/bin/sh");
        assert_eq!(shell_program(Some("/bin/fish".to_string())), "/bin/fish");
    }

    #[test]
    fn stable_profile_names_and_legacy_indices_are_supported() {
        for (index, profile) in PROFILES.iter().enumerate() {
            assert_eq!(parse_profile(profile.name), Ok((index, false)));
            assert_eq!(parse_profile(&index.to_string()), Ok((index, true)));
        }
    }

    #[test]
    fn malformed_or_unknown_profiles_are_reported() {
        assert!(parse_profile("").is_err());
        assert!(parse_profile("Not a Profile").is_err());
        assert!(parse_profile(&PROFILES.len().to_string()).is_err());
    }
}
