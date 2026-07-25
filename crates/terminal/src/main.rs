//! rmac Terminal — a fast, native terminal emulator.
//!
//! The Zed stack: `alacritty_terminal` drives the grid/escape-sequence state,
//! `portable-pty` runs the user's shell, and GPUI renders the cell grid. A
//! background thread reads PTY output and feeds the parser; model changes wake
//! the view, which renders the grid and writes keystrokes back to the PTY.

mod storage;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{sync_channel, SyncSender},
    Arc, Mutex,
};
use std::thread::JoinHandle;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Row, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, ClipboardItem, Context, Div, Entity,
    FocusHandle, Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize,
};
use rmac_ui::{Button, InputState, SearchField};
use vte::ansi::{ClearMode, Color, Handler as _, NamedColor, Processor};

type RedrawSender = async_channel::Sender<()>;

const COLS: usize = 100;
const ROWS: usize = 28;
const MIN_COLS: usize = 20;
const MIN_ROWS: usize = 5;
const MAX_COLS: usize = 500;
const MAX_ROWS: usize = 300;
const MAX_TABS: usize = 16;
const SCROLLBACK_LINES: usize = 10_000;
const MAX_GRID_BASE_BYTES_PER_WINDOW: usize = 512 * 1024 * 1024;
/// `Vec` can retain almost twice the requested elements after amortized growth.
const MAX_ROW_CELL_CAPACITY_FACTOR: usize = 2;
/// Alacritty retains both the primary and alternate visible screen grids.
const VISIBLE_GRID_COPIES: usize = 2;
/// Conservative allocator metadata/alignment/size-class allowance per cell row.
const ROW_ALLOCATION_ALLOWANCE_BYTES: usize = 1024;
/// Reserve for retained/rounded primary and alternate outer Row storage.
const OUTER_ROW_STORAGE_ALLOWANCE_BYTES_PER_TAB: usize = 1024 * 1024;
const FONT: &str = "Menlo"; // macOS Terminal's default monospace
const FONT_SIZE: f32 = 13.0;
const LINE_H: f32 = 17.0;
/// Approximate monospace cell advance for `Menlo` at `FONT_SIZE`.
const CELL_W: f32 = FONT_SIZE * 0.6;
/// Pixels from the window top to the first text row: 34pt title bar + 8pt pad.
const TOP_PAD: f32 = 34.0 + 8.0;
/// Pixels from the window left to the first column: 8pt content padding.
const LEFT_PAD: f32 = 8.0;
const MAX_PASTE_BYTES: usize = 1024 * 1024;
const MAX_OSC_PAYLOAD_BYTES: usize = 1024;
const MAX_COMBINING_MARKS_PER_CELL: usize = 16;
/// One reader and one child waiter are reserved before a shell can launch.
#[cfg(test)]
const SESSION_WORKERS_PER_TAB: usize = 2;
/// Parser storage is heap-backed; 512 KiB is ample for each shallow worker.
const SESSION_WORKER_STACK_BYTES: usize = 512 * 1024;
/// `vte` 0.15 terminates synchronized updates before this heap buffer fills.
#[cfg(test)]
const MAX_VTE_SYNC_BUFFER_BYTES: usize = 2 * 1024 * 1024;
#[cfg(test)]
const MAX_VTE_CSI_PARAMETERS: usize = 32;
#[cfg(test)]
const MAX_VTE_INTERMEDIATES: usize = 2;
#[cfg(test)]
const MAX_UTF8_SCALAR_BYTES: usize = 4;
/// `alacritty_terminal` 0.25 evicts the oldest saved title at this depth.
#[cfg(test)]
const MAX_TITLE_STACK_DEPTH: usize = 4096;
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn terminal_config(scrollback_lines: usize) -> Config {
    Config {
        scrolling_history: scrollback_lines.min(SCROLLBACK_LINES),
        ..Config::default()
    }
}

fn bounded_cell_row_bytes() -> usize {
    MAX_COLS
        .saturating_mul(MAX_ROW_CELL_CAPACITY_FACTOR)
        .saturating_mul(std::mem::size_of::<Cell>())
        .saturating_add(ROW_ALLOCATION_ALLOWANCE_BYTES)
}

fn retained_row_slots_bytes_per_tab() -> usize {
    // Shrinking history drops each Row's cell allocation, but the outer
    // primary/alternate Vecs can retain their old Row-slot capacities. Budget
    // their lifetime maximum independently from the current history limit.
    SCROLLBACK_LINES
        .saturating_add(MAX_ROWS.saturating_mul(VISIBLE_GRID_COPIES))
        .saturating_mul(MAX_ROW_CELL_CAPACITY_FACTOR)
        .saturating_mul(std::mem::size_of::<Row<Cell>>())
        .saturating_add(OUTER_ROW_STORAGE_ALLOWANCE_BYTES_PER_TAB)
}

fn scrollback_limit_for_tab_count(tab_count: usize) -> usize {
    if tab_count == 0 {
        return 0;
    }
    let limit = MAX_GRID_BASE_BYTES_PER_WINDOW
        .checked_div(tab_count)
        .unwrap_or(0)
        .saturating_sub(retained_row_slots_bytes_per_tab())
        .checked_div(bounded_cell_row_bytes())
        .unwrap_or(0)
        .saturating_sub(MAX_ROWS.saturating_mul(VISIBLE_GRID_COPIES))
        .min(SCROLLBACK_LINES);
    debug_assert!(
        bounded_grid_base_bytes(tab_count, limit) <= MAX_GRID_BASE_BYTES_PER_WINDOW,
        "scrollback history must stay inside the base-grid budget"
    );
    limit
}

fn bounded_grid_base_bytes(tab_count: usize, history_lines: usize) -> usize {
    let cell_rows = tab_count
        .saturating_mul(
            MAX_ROWS
                .saturating_mul(VISIBLE_GRID_COPIES)
                .saturating_add(history_lines),
        )
        .saturating_mul(bounded_cell_row_bytes());
    cell_rows.saturating_add(tab_count.saturating_mul(retained_row_slots_bytes_per_tab()))
}

#[cfg(test)]
fn max_session_worker_stack_bytes_per_window() -> usize {
    MAX_TABS
        .saturating_mul(SESSION_WORKERS_PER_TAB)
        .saturating_mul(SESSION_WORKER_STACK_BYTES)
}

#[derive(Debug, Default)]
enum OutputFilterState {
    #[default]
    Ground,
    Escape,
    Osc {
        bytes: Vec<u8>,
        payload_bytes: usize,
        overflowed: bool,
    },
    OscEscape {
        bytes: Vec<u8>,
        overflowed: bool,
    },
}

#[derive(Debug, Default)]
struct OutputFilter {
    state: OutputFilterState,
}

impl OutputFilter {
    fn osc_ignored_control(byte: u8) -> bool {
        matches!(byte, 0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f)
    }

    fn osc_command(bytes: &[u8]) -> Option<u16> {
        let payload = bytes.strip_prefix(b"\x1b]")?;
        let mut command = 0u16;
        let mut found_digit = false;
        for &byte in payload {
            if Self::osc_ignored_control(byte) {
                continue;
            }
            if byte == b';' {
                return found_digit.then_some(command);
            }
            if !byte.is_ascii_digit() {
                return None;
            }
            found_digit = true;
            command = command
                .checked_mul(10)?
                .checked_add(u16::from(byte - b'0'))?;
        }
        None
    }

    fn emit_osc(bytes: &[u8], overflowed: bool, output: &mut Vec<u8>) {
        // OSC 8 hyperlinks remain disabled until Terminal has a reviewed
        // activation/display policy. This also prevents unrendered hyperlink
        // metadata from occupying per-cell dynamic storage.
        if !overflowed && Self::osc_command(bytes) != Some(8) {
            output.extend_from_slice(bytes);
        }
    }

    fn push_osc_payload(
        bytes: &mut Vec<u8>,
        payload_bytes: &mut usize,
        overflowed: &mut bool,
        byte: u8,
    ) {
        if *overflowed {
            return;
        }
        if *payload_bytes >= MAX_OSC_PAYLOAD_BYTES {
            bytes.clear();
            *overflowed = true;
            return;
        }
        bytes.push(byte);
        *payload_bytes += 1;
    }

    /// Copy a PTY chunk into `output`, buffering OSC title/hyperlink sequences
    /// until their terminator. Allowed valid sequences are preserved
    /// byte-for-byte; hyperlinks, overlong, malformed, and unterminated
    /// sequences never reach VTE's otherwise growable standard-library OSC
    /// buffer.
    fn filter_into(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.clear();
        for &byte in input {
            let state = std::mem::take(&mut self.state);
            self.state = match state {
                OutputFilterState::Ground if byte == 0x1b => OutputFilterState::Escape,
                OutputFilterState::Ground => {
                    output.push(byte);
                    OutputFilterState::Ground
                }
                OutputFilterState::Escape if byte == b']' => OutputFilterState::Osc {
                    bytes: vec![0x1b, b']'],
                    payload_bytes: 0,
                    overflowed: false,
                },
                OutputFilterState::Escape if matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f) => {
                    // These controls execute without leaving VTE's Escape
                    // state. Emit the state-independent control now while
                    // retaining the Escape introducer for OSC detection.
                    output.push(byte);
                    OutputFilterState::Escape
                }
                OutputFilterState::Escape if matches!(byte, 0x18 | 0x1a) => {
                    output.push(byte);
                    OutputFilterState::Ground
                }
                OutputFilterState::Escape if byte == 0x1b => {
                    output.push(0x1b);
                    OutputFilterState::Escape
                }
                OutputFilterState::Escape => {
                    output.extend_from_slice(&[0x1b, byte]);
                    OutputFilterState::Ground
                }
                OutputFilterState::Osc {
                    mut bytes,
                    payload_bytes: _,
                    overflowed,
                } if byte == 0x07 => {
                    if !overflowed {
                        bytes.push(byte);
                    }
                    Self::emit_osc(&bytes, overflowed, output);
                    OutputFilterState::Ground
                }
                OutputFilterState::Osc {
                    bytes,
                    payload_bytes: _,
                    overflowed,
                } if matches!(byte, 0x18 | 0x1a) => {
                    Self::emit_osc(&bytes, overflowed, output);
                    output.push(byte);
                    OutputFilterState::Ground
                }
                OutputFilterState::Osc {
                    bytes,
                    payload_bytes: _,
                    overflowed,
                } if byte == 0x1b => OutputFilterState::OscEscape { bytes, overflowed },
                OutputFilterState::Osc {
                    mut bytes,
                    mut payload_bytes,
                    mut overflowed,
                } => {
                    Self::push_osc_payload(&mut bytes, &mut payload_bytes, &mut overflowed, byte);
                    OutputFilterState::Osc {
                        bytes,
                        payload_bytes,
                        overflowed,
                    }
                }
                OutputFilterState::OscEscape {
                    mut bytes,
                    overflowed,
                } if byte == b'\\' => {
                    if !overflowed {
                        bytes.extend_from_slice(&[0x1b, b'\\']);
                    }
                    Self::emit_osc(&bytes, overflowed, output);
                    OutputFilterState::Ground
                }
                OutputFilterState::OscEscape { .. } if byte == 0x1b => {
                    // The previous OSC is malformed and discarded; this new
                    // Escape can still begin a fresh, independently bounded one.
                    OutputFilterState::Escape
                }
                OutputFilterState::OscEscape { .. } => {
                    // An embedded non-ST Escape makes the OSC malformed. Drop
                    // the buffered sequence as a unit, then resume ordinary
                    // ground-state output with the current byte.
                    output.push(byte);
                    OutputFilterState::Ground
                }
            };
        }
    }

    #[cfg(test)]
    fn buffered_bytes(&self) -> usize {
        match &self.state {
            OutputFilterState::Osc { bytes, .. } | OutputFilterState::OscEscape { bytes, .. } => {
                bytes.len()
            }
            OutputFilterState::Ground | OutputFilterState::Escape => 0,
        }
    }
}

fn cap_cursor_combining_marks<T: EventListener>(term: &mut Term<T>) {
    let (line, mut column, input_needs_wrap) = {
        let grid = term.grid();
        (
            grid.cursor.point.line,
            grid.cursor.point.column,
            grid.cursor.input_needs_wrap,
        )
    };
    if !input_needs_wrap {
        column.0 = column.0.saturating_sub(1);
    }
    if term.grid()[line][column]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER)
    {
        column.0 = column.0.saturating_sub(1);
    }

    let cell = &term.grid()[line][column];
    let Some(zerowidth) = cell
        .zerowidth()
        .filter(|marks| marks.len() > MAX_COMBINING_MARKS_PER_CELL)
    else {
        return;
    };
    let retained = zerowidth[..MAX_COMBINING_MARKS_PER_CELL].to_vec();
    let mut bounded = Cell {
        c: cell.c,
        fg: cell.fg,
        bg: cell.bg,
        flags: cell.flags,
        extra: None,
    };
    bounded.set_underline_color(cell.underline_color());
    bounded.set_hyperlink(cell.hyperlink());
    for mark in retained {
        bounded.push_zerowidth(mark);
    }
    term.grid_mut()[line][column] = bounded;
}

fn advance_filtered_output<T: EventListener>(
    parser: &mut Processor,
    term: &mut Term<T>,
    bytes: &[u8],
) {
    let mut segment_start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        // Non-ASCII bytes can complete a zero-width scalar. ASCII `b` can
        // complete CSI REP and repeat the preceding combining scalar up to a
        // u16 count, so it is also an immediate cap boundary.
        if byte & 0x80 == 0 && *byte != b'b' {
            continue;
        }
        parser.advance(term, &bytes[segment_start..=index]);
        cap_cursor_combining_marks(term);
        segment_start = index + 1;
    }
    if segment_start < bytes.len() {
        parser.advance(term, &bytes[segment_start..]);
        cap_cursor_combining_marks(term);
    }
}

fn grid_dimensions(width: f32, height: f32, cell_width: f32, line_height: f32) -> TermSize {
    let bounded = |available: f32, cell: f32, minimum: usize| {
        if !available.is_finite() || !cell.is_finite() || cell <= 0.0 {
            return minimum;
        }
        (available.max(0.0) / cell).floor() as usize
    };
    TermSize {
        cols: bounded(width - 16.0, cell_width, MIN_COLS).clamp(MIN_COLS, MAX_COLS),
        lines: bounded(height - 50.0, line_height, MIN_ROWS).clamp(MIN_ROWS, MAX_ROWS),
    }
}

fn logical_line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut lines = 1;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                lines += 1;
                index += 2;
            }
            b'\r' | b'\n' => {
                lines += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    lines
}

fn has_unsafe_unbracketed_control(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\r' | '\n'))
}

fn prepare_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(text.len().saturating_add(12));
        bytes.extend_from_slice(BRACKETED_PASTE_START);
        bytes.extend(
            text.as_bytes()
                .iter()
                .copied()
                .filter(|byte| !matches!(byte, b'\x1b' | b'\x03')),
        );
        bytes.extend_from_slice(BRACKETED_PASTE_END);
        bytes
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// xterm's one-based modifier parameter: Shift=1, Alt=2, Control=4.
fn xterm_modifier(modifiers: &gpui::Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control)
}

fn cursor_key_sequence(
    final_byte: char,
    modifiers: &gpui::Modifiers,
    application_cursor: bool,
) -> Vec<u8> {
    let modifier = xterm_modifier(modifiers);
    if modifier != 1 {
        format!("\x1b[1;{modifier}{final_byte}").into_bytes()
    } else if application_cursor {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

fn tilde_key_sequence(code: u8, modifiers: &gpui::Modifiers) -> Vec<u8> {
    let modifier = xterm_modifier(modifiers);
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

fn function_key_sequence(key: &str, modifiers: &gpui::Modifiers) -> Option<Vec<u8>> {
    let modifier = xterm_modifier(modifiers);
    let sequence = match key {
        "f1" | "f2" | "f3" | "f4" => {
            let final_byte = match key {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if modifier == 1 {
                format!("\x1bO{final_byte}").into_bytes()
            } else {
                format!("\x1b[1;{modifier}{final_byte}").into_bytes()
            }
        }
        "f5" => tilde_key_sequence(15, modifiers),
        "f6" => tilde_key_sequence(17, modifiers),
        "f7" => tilde_key_sequence(18, modifiers),
        "f8" => tilde_key_sequence(19, modifiers),
        "f9" => tilde_key_sequence(20, modifiers),
        "f10" => tilde_key_sequence(21, modifiers),
        "f11" => tilde_key_sequence(23, modifiers),
        "f12" => tilde_key_sequence(24, modifiers),
        "f13" => tilde_key_sequence(25, modifiers),
        "f14" => tilde_key_sequence(26, modifiers),
        "f15" => tilde_key_sequence(28, modifiers),
        "f16" => tilde_key_sequence(29, modifiers),
        "f17" => tilde_key_sequence(31, modifiers),
        "f18" => tilde_key_sequence(32, modifiers),
        "f19" => tilde_key_sequence(33, modifiers),
        "f20" => tilde_key_sequence(34, modifiers),
        _ => return None,
    };
    Some(sequence)
}

fn is_supported_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=20).contains(&number))
}

fn control_byte(key: &str) -> Option<u8> {
    let character = key.chars().next()?;
    if key.len() == 1 && character.is_ascii_alphabetic() {
        return Some((character.to_ascii_uppercase() as u8) & 0x1f);
    }
    match key {
        "space" | "@" | "2" => Some(0x00),
        "[" | "{" | "3" => Some(0x1b),
        "\\" | "|" | "4" => Some(0x1c),
        "]" | "}" | "5" => Some(0x1d),
        "^" | "~" | "6" => Some(0x1e),
        "_" | "/" | "7" => Some(0x1f),
        "?" | "8" => Some(0x7f),
        _ => None,
    }
}

/// Encode the traditional xterm/DEC key contract represented by GPUI.
///
/// GPUI intentionally does not preserve numeric-keypad location, so APP_KEYPAD
/// remains a separate framework gate. Enhanced Kitty keyboard modes likewise
/// require key release/location metadata beyond this key-down path.
fn encode_key(keystroke: &gpui::Keystroke, mode: TermMode) -> Vec<u8> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform {
        return Vec::new();
    }

    let application_cursor = mode.contains(TermMode::APP_CURSOR);
    let mut bytes = match keystroke.key.as_str() {
        "enter" => vec![b'\r'],
        "backspace" => vec![0x7f],
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        "escape" => vec![0x1b],
        "up" => cursor_key_sequence('A', modifiers, application_cursor),
        "down" => cursor_key_sequence('B', modifiers, application_cursor),
        "right" => cursor_key_sequence('C', modifiers, application_cursor),
        "left" => cursor_key_sequence('D', modifiers, application_cursor),
        "home" => cursor_key_sequence('H', modifiers, application_cursor),
        "end" => cursor_key_sequence('F', modifiers, application_cursor),
        "insert" => tilde_key_sequence(2, modifiers),
        "delete" => tilde_key_sequence(3, modifiers),
        "pageup" => tilde_key_sequence(5, modifiers),
        "pagedown" => tilde_key_sequence(6, modifiers),
        key if is_supported_function_key(key) => {
            return function_key_sequence(key, modifiers).unwrap_or_default();
        }
        key if modifiers.control => control_byte(key).into_iter().collect(),
        _ => keystroke
            .key_char
            .as_deref()
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };

    // xterm's conventional Meta/Alt behavior prefixes ordinary and control
    // characters with Escape. Special cursor/function keys encode Alt in
    // their modifier parameter instead.
    let parameterized_special = matches!(
        keystroke.key.as_str(),
        "up" | "down"
            | "right"
            | "left"
            | "home"
            | "end"
            | "insert"
            | "delete"
            | "pageup"
            | "pagedown"
    );
    if modifiers.alt && !parameterized_special && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum SessionLifecycle {
    Running,
    Exited {
        exit_code: u32,
        signal: Option<String>,
    },
    WaitFailed,
    StartFailed,
}

impl SessionLifecycle {
    fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    fn may_be_running(&self) -> bool {
        matches!(self, Self::Running | Self::WaitFailed)
    }

    fn status_message(&self) -> Option<String> {
        match self {
            Self::Running => None,
            Self::Exited {
                exit_code: 0,
                signal: None,
            } => Some("The shell exited successfully.".into()),
            Self::Exited {
                signal: Some(signal),
                ..
            } => Some(format!("The shell was terminated by {signal}.")),
            Self::Exited { exit_code, .. } => {
                Some(format!("The shell exited with status {exit_code}."))
            }
            Self::WaitFailed => Some("Terminal could not observe the shell's exit status.".into()),
            Self::StartFailed => Some("Terminal could not start the configured shell.".into()),
        }
    }

    fn tab_state_label(&self) -> Option<&'static str> {
        match self {
            Self::Running => None,
            Self::Exited { .. } => Some("Exited"),
            Self::WaitFailed | Self::StartFailed => Some("Unavailable"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionStartError {
    OpenPty,
    StartShell,
    OpenReader,
    OpenWriter,
    StartReaderWorker,
    StartWaiterWorker,
}

impl std::fmt::Display for SessionStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::OpenPty => "Terminal could not create a private terminal session.",
            Self::StartShell => "Terminal could not start the configured shell.",
            Self::OpenReader => "Terminal could not receive output from the shell.",
            Self::OpenWriter => "Terminal could not send input to the shell.",
            Self::StartReaderWorker | Self::StartWaiterWorker => {
                "Terminal could not reserve bounded resources for the configured shell."
            }
        })
    }
}

impl std::error::Error for SessionStartError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionControlError;

impl std::fmt::Display for SessionControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Terminal could not terminate the selected shell safely.")
    }
}

impl std::error::Error for SessionControlError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionWriteError {
    Exited,
    State,
    Write,
}

impl std::fmt::Display for SessionWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Exited => "This terminal session is no longer accepting input.",
            Self::State => "Terminal could not safely access the session state.",
            Self::Write => "Terminal could not send input to the shell.",
        })
    }
}

impl std::error::Error for SessionWriteError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteError {
    ReviewRequired,
    UnsafeControl,
    Session(SessionWriteError),
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ReviewRequired => "This multiline paste requires review.",
            Self::UnsafeControl => {
                "Paste contains control characters the active program did not protect."
            }
            Self::Session(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for PasteError {}

fn lifecycle_after_wait(result: std::io::Result<ExitStatus>) -> SessionLifecycle {
    match result {
        Ok(status) => SessionLifecycle::Exited {
            exit_code: status.exit_code(),
            signal: status.signal().map(str::to_string),
        },
        Err(_) => SessionLifecycle::WaitFailed,
    }
}

fn foreground_job_requires_confirmation(
    running: bool,
    shell_pid: Option<u32>,
    foreground_process_group: Option<u32>,
) -> bool {
    running
        && shell_pid
            .zip(foreground_process_group)
            .is_none_or(|(shell, foreground)| shell != foreground)
}

type ReaderTask = (
    Box<dyn Read + Send>,
    Arc<Mutex<Term<EventProxy>>>,
    RedrawSender,
);
type WaiterTask = (
    Box<dyn Child + Send + Sync>,
    Arc<Mutex<SessionLifecycle>>,
    RedrawSender,
);

fn run_reader_worker((mut reader, term, redraw): ReaderTask) {
    let mut parser: Processor = Processor::new();
    let mut output_filter = OutputFilter::default();
    let mut buf = [0u8; 8192];
    let mut filtered = Vec::with_capacity(buf.len());
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                output_filter.filter_into(&buf[..n], &mut filtered);
                if filtered.is_empty() {
                    continue;
                }
                if let Ok(mut term) = term.lock() {
                    advance_filtered_output(&mut parser, &mut *term, &filtered);
                }
                request_redraw(&redraw);
            }
        }
    }
    request_redraw(&redraw);
}

fn run_waiter_worker((mut child, lifecycle, redraw): WaiterTask) {
    let next = lifecycle_after_wait(child.wait());
    if let Ok(mut lifecycle) = lifecycle.lock() {
        *lifecycle = next;
    }
    request_redraw(&redraw);
}

fn reserve_session_worker<Task, Run>(
    name: &'static str,
    run: Run,
) -> std::io::Result<(SyncSender<Task>, JoinHandle<()>)>
where
    Task: Send + 'static,
    Run: FnOnce(Task) + Send + 'static,
{
    let (sender, receiver) = sync_channel(0);
    let handle = std::thread::Builder::new()
        .name(name.into())
        .stack_size(SESSION_WORKER_STACK_BYTES)
        .spawn(move || {
            if let Ok(task) = receiver.recv() {
                run(task);
            }
        })?;
    Ok((sender, handle))
}

struct ReservedSessionWorkers {
    reader_sender: SyncSender<ReaderTask>,
    reader_handle: JoinHandle<()>,
    waiter_sender: SyncSender<WaiterTask>,
    waiter_handle: JoinHandle<()>,
}

impl ReservedSessionWorkers {
    /// Reserve both fallible OS threads before the shell exists. Dropping the
    /// rendezvous sender makes an unused worker exit without polling.
    fn reserve() -> Result<Self, SessionStartError> {
        let (reader_sender, reader_handle) =
            reserve_session_worker("rmac-terminal-reader", run_reader_worker)
                .map_err(|_| SessionStartError::StartReaderWorker)?;
        let (waiter_sender, waiter_handle) =
            match reserve_session_worker("rmac-terminal-waiter", run_waiter_worker) {
                Ok(worker) => worker,
                Err(_) => {
                    drop(reader_sender);
                    let _ = reader_handle.join();
                    return Err(SessionStartError::StartWaiterWorker);
                }
            };
        Ok(Self {
            reader_sender,
            reader_handle,
            waiter_sender,
            waiter_handle,
        })
    }

    fn activate(
        self,
        reader_task: ReaderTask,
        waiter_task: WaiterTask,
        killer: &mut dyn ChildKiller,
    ) -> Result<(), SessionStartError> {
        let Self {
            reader_sender,
            reader_handle,
            waiter_sender,
            waiter_handle,
        } = self;

        // Supervision starts first. If this reserved receiver disappeared,
        // retain the returned child task and synchronously reap it.
        if let Err(error) = waiter_sender.send(waiter_task) {
            let (mut child, _, _) = error.0;
            let _ = child.kill();
            let _ = child.wait();
            drop(reader_sender);
            let _ = reader_handle.join();
            let _ = waiter_handle.join();
            return Err(SessionStartError::StartWaiterWorker);
        }

        // The waiter now owns the child. A theoretically disconnected reader
        // fails closed by terminating that child; the waiter remains attached
        // long enough to reap it.
        if reader_sender.send(reader_task).is_err() {
            let _ = killer.kill();
            let _ = reader_handle.join();
            drop(waiter_handle);
            return Err(SessionStartError::StartReaderWorker);
        }

        // Dropping JoinHandle detaches the two bounded lifetime workers.
        drop(reader_handle);
        drop(waiter_handle);
        Ok(())
    }
}

/// One terminal tab: its own PTY + parser-fed grid. `master` is `None` for a
/// failed session (PTY/shell couldn't start) — it still renders an error grid.
struct Session {
    id: u64,
    term: Arc<Mutex<Term<EventProxy>>>,
    writer: Box<dyn Write + Send>,
    master: Option<Box<dyn MasterPty + Send>>,
    shell_pid: Option<u32>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    lifecycle: Arc<Mutex<SessionLifecycle>>,
}

impl Session {
    /// Start a real shell in a PTY. Returns a private-safe typed failure
    /// instead of panicking or exposing environment-derived shell details.
    fn spawn(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        redraw: RedrawSender,
    ) -> Result<Session, SessionStartError> {
        let size = TermSize { cols, lines: rows };
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| SessionStartError::OpenPty)?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|_| SessionStartError::OpenReader)?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|_| SessionStartError::OpenWriter)?;
        let workers = ReservedSessionWorkers::reserve()?;
        let shell = shell_program(std::env::var("SHELL").ok());
        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|_| SessionStartError::StartShell)?;
        let shell_pid = child.process_id();
        let mut killer = child.clone_killer();
        drop(pair.slave);
        let term = Arc::new(Mutex::new(Term::new(
            terminal_config(scrollback_lines),
            &size,
            EventProxy,
        )));
        let lifecycle = Arc::new(Mutex::new(SessionLifecycle::Running));
        workers.activate(
            (reader, term.clone(), redraw.clone()),
            (child, lifecycle.clone(), redraw),
            killer.as_mut(),
        )?;
        Ok(Session {
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            term,
            writer,
            master: Some(pair.master),
            shell_pid,
            killer: Some(killer),
            lifecycle,
        })
    }

    /// A no-PTY session that just displays an error message in its grid, so a
    /// shell-startup failure degrades gracefully instead of crashing.
    fn failed(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        error: SessionStartError,
    ) -> Session {
        let size = TermSize { cols, lines: rows };
        let term = Arc::new(Mutex::new(Term::new(
            terminal_config(scrollback_lines),
            &size,
            EventProxy,
        )));
        if let Ok(mut t) = term.lock() {
            let mut parser: Processor = Processor::new();
            let text = format!("\r\n  {error}\r\n");
            parser.advance(&mut *t, text.as_bytes());
        }
        Session {
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            term,
            writer: Box::new(std::io::sink()),
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::StartFailed)),
        }
    }

    fn lifecycle(&self) -> SessionLifecycle {
        self.lifecycle
            .lock()
            .map(|lifecycle| lifecycle.clone())
            .unwrap_or(SessionLifecycle::WaitFailed)
    }

    fn has_foreground_job(&self) -> bool {
        let may_be_running = self.lifecycle().may_be_running();
        #[cfg(unix)]
        let foreground_process_group = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .and_then(|pid| u32::try_from(pid).ok());
        #[cfg(not(unix))]
        let foreground_process_group = None;
        foreground_job_requires_confirmation(
            may_be_running,
            self.shell_pid,
            foreground_process_group,
        )
    }

    fn terminate(&mut self) -> Result<(), SessionControlError> {
        if !self.lifecycle().may_be_running() {
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(process_group) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .filter(|process_group| *process_group > 0)
            .filter(|process_group| u32::try_from(*process_group).ok() != self.shell_pid)
        {
            // SAFETY: `process_group` is the positive foreground group returned
            // by this still-owned PTY. Negating it requests SIGHUP for that
            // exact group and does not dereference application memory.
            let result = unsafe { libc::kill(-process_group, libc::SIGHUP) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(SessionControlError);
            }
        }
        self.killer
            .as_mut()
            .ok_or(SessionControlError)?
            .kill()
            .map_err(|_| SessionControlError)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SessionWriteError> {
        if !self.lifecycle().is_running() {
            return Err(SessionWriteError::Exited);
        }
        self.writer
            .write_all(bytes)
            .and_then(|_| self.writer.flush())
            .map_err(|_| SessionWriteError::Write)
    }

    fn paste(&mut self, text: &str, reviewed_multiline: bool) -> Result<(), PasteError> {
        if !self.lifecycle().is_running() {
            return Err(PasteError::Session(SessionWriteError::Exited));
        }
        let mut term = self
            .term
            .lock()
            .map_err(|_| PasteError::Session(SessionWriteError::State))?;
        let bracketed = term.mode().contains(TermMode::BRACKETED_PASTE);
        if !bracketed {
            if has_unsafe_unbracketed_control(text) {
                return Err(PasteError::UnsafeControl);
            }
            if logical_line_count(text) > 1 && !reviewed_multiline {
                return Err(PasteError::ReviewRequired);
            }
        }
        term.scroll_display(Scroll::Bottom);
        drop(term);
        self.write(&prepare_paste(text, bracketed))
            .map_err(PasteError::Session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.lifecycle().may_be_running() {
            let _ = self.killer.as_mut().map(|killer| killer.kill());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingClose {
    Tab { session_id: u64 },
    Window { foreground_sessions: usize },
}

struct PendingPaste {
    session_id: u64,
    text: String,
    line_count: usize,
    byte_count: usize,
}

impl std::fmt::Debug for PendingPaste {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingPaste")
            .field("session_id", &self.session_id)
            .field("text", &"<private>")
            .field("line_count", &self.line_count)
            .field("byte_count", &self.byte_count)
            .finish()
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
    operation_error: Option<SharedString>,
    pending_close: Option<PendingClose>,
    pending_paste: Option<PendingPaste>,
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
        let scrollback_lines = scrollback_limit_for_tab_count(1);
        let session = Session::spawn(COLS, ROWS, scrollback_lines, redraw.clone())
            .unwrap_or_else(|error| Session::failed(COLS, ROWS, scrollback_lines, error));

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

    fn new_tab(&mut self, cx: &mut Context<Self>) {
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
        let (c, r) = (self.cols.max(MIN_COLS), self.rows.max(MIN_ROWS));
        self.tabs.push(
            Session::spawn(c, r, scrollback_lines, self.redraw.clone())
                .unwrap_or_else(|error| Session::failed(c, r, scrollback_lines, error)),
        );
        self.active = self.tabs.len() - 1;
        self.selection = None;
        self.cols = 0; // force resize_to() to re-fit the new active session
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
            self.searching = false;
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
        self.remove_tab(session_id, cx);
    }

    fn remove_tab(&mut self, session_id: u64, cx: &mut Context<Self>) {
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
        self.tabs.remove(index);
        if self.active > index {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.selection = None;
        self.cols = 0;
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
        self.searching = false;
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
                self.remove_tab(session_id, cx);
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

    fn select_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.modal_open() || i >= self.tabs.len() {
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
        let history_limit = scrollback_limit_for_tab_count(n);
        let mut bar = div()
            .h(px(32.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(rmac_ui::mac::separator());
        for i in 0..n {
            let is_active = i == active_tab;
            let lifecycle = self.tabs[i].lifecycle();
            let label = lifecycle.tab_state_label().map_or_else(
                || format!("Terminal {}", i + 1),
                |state| format!("Terminal {} — {state}", i + 1),
            );
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(26.0))
                    .px_2()
                    .rounded(px(5.0))
                    .when(is_active, |el: Div| el.bg(hsla(active().bg)))
                    .child(
                        div()
                            .id(("tabname", i))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(if is_active {
                                hsla(active().fg)
                            } else {
                                rmac_ui::mac::text_secondary()
                            })
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(("tabclose", i))
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(rmac_ui::mac::text_secondary())
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_close_tab(i, window, cx);
                            })),
                    ),
            );
        }
        bar.child(div().flex_1())
            .child(
                div()
                    .px_1()
                    .text_size(rmac_ui::text_px(10.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child(format!("{history_limit} history lines/tab")),
            )
            .child(
                div()
                    .id("newtab")
                    .w(px(22.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .text_size(rmac_ui::text_px(15.0))
                    .text_color(rmac_ui::mac::text_secondary())
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
        Button::new("profile-chip", format!("{name}  ▼"))
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
            .bg(rmac_ui::mac::window())
            .rounded(px(8.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
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
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::text())
                    .hover(|h| {
                        h.bg(rmac_ui::mac::accent())
                            .text_color(rmac_ui::mac::on_accent())
                    })
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(rmac_ui::mac::separator())
                            .bg(hsla(p.bg)),
                    )
                    .child(div().flex_1().child(p.name))
                    .when(is_active, |el: Stateful<Div>| {
                        el.child(div().text_color(rmac_ui::mac::accent()).child("✓"))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_profile(i, cx)))
            }))
    }

    fn render_close_confirmation(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let pending = self.pending_close?;
        let (title, message, confirm_label): (&str, String, &str) = match pending {
            PendingClose::Tab { .. } => (
                "Close this terminal tab?",
                "A foreground process group is still using this terminal. Closing sends hangup to that group and its shell, then removes the tab.".into(),
                "Close Tab",
            ),
            PendingClose::Window {
                foreground_sessions,
            } => (
                "Close this Terminal window?",
                if foreground_sessions == 1 {
                    "One tab has an active foreground process group. Closing sends hangup to active groups and shells, then removes the window.".into()
                } else {
                    format!(
                        "{foreground_sessions} tabs have active foreground process groups. Closing sends hangup to active groups and shells, then removes the window."
                    )
                },
                "Close Window",
            ),
        };
        Some(rmac_ui::alert(
            title,
            message,
            vec![
                rmac_ui::dialog_button("terminal-close-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_close(window, cx);
                    }))
                    .into_any_element(),
                rmac_ui::dialog_button("terminal-close-confirm", confirm_label, Destructive)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_close(window, cx);
                    }))
                    .into_any_element(),
            ],
        ))
    }

    fn render_paste_confirmation(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let pending = self.pending_paste.as_ref()?;
        let message = format!(
            "The clipboard contains {} lines ({} bytes), but the active program did not enable bracketed paste. Continuing sends line breaks as Return and may run commands.",
            pending.line_count, pending.byte_count
        );
        Some(rmac_ui::alert(
            "Paste multiple lines?",
            message,
            vec![
                rmac_ui::dialog_button("terminal-paste-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_paste(window, cx);
                    }))
                    .into_any_element(),
                rmac_ui::dialog_button("terminal-paste-confirm", "Paste Anyway", Destructive)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_paste(window, cx);
                    }))
                    .into_any_element(),
            ],
        ))
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
        self.selection = None;
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
        if self.modal_open() {
            return;
        }
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
        let size = grid_dimensions(w, h, self.cell_w, self.line_h);
        let cols = size.cols;
        let rows = size.lines;
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

    fn on_key(&mut self, ev: &KeyDownEvent) -> Result<(), SessionWriteError> {
        let mut term = self.tabs[self.active]
            .term
            .lock()
            .map_err(|_| SessionWriteError::State)?;
        let bytes = encode_key(&ev.keystroke, *term.mode());
        if bytes.is_empty() {
            return Ok(());
        }
        // Typing jumps to the live prompt and clears the visual selection.
        term.scroll_display(Scroll::Bottom);
        drop(term);
        self.selection = None;
        self.tabs[self.active].write(&bytes)
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
                self.pending_paste = Some(PendingPaste {
                    session_id: self.tabs[self.active].id,
                    line_count: logical_line_count(&text),
                    byte_count: text.len(),
                    text,
                });
                self.searching = false;
                self.picker_open = false;
                self.menu_at = None;
                window.focus(&self.focus);
            }
            Err(error) => self.operation_error = Some(error.to_string().into()),
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
            self.operation_error = Some(error.to_string().into());
        }
        window.focus(&self.focus);
        cx.notify();
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
        // Hide the cursor while scrolled into history or after the child exits;
        // an exited session must not resemble a live prompt.
        let show_cursor = offset == 0 && self.tabs[self.active].lifecycle().is_running();
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
        let operation_error_visible = self.operation_error.is_some();
        let terminal_error = self
            .operation_error
            .clone()
            .or_else(|| self.persistence_error.clone());
        let has_terminal_error = terminal_error.is_some();
        let session_status = self.tabs[self.active]
            .lifecycle()
            .status_message()
            .map(SharedString::from);
        let close_confirmation = self
            .render_close_confirmation(cx)
            .map(|alert| alert.into_any_element());
        let paste_confirmation = self
            .render_paste_confirmation(cx)
            .map(|alert| alert.into_any_element());
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
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if this.modal_open() {
                            if ev.keystroke.key == "escape" {
                                if this.pending_close.is_some() {
                                    this.cancel_close(window, cx);
                                } else {
                                    this.cancel_paste(window, cx);
                                }
                            }
                            return;
                        }
                        if let Err(error) = this.on_key(ev) {
                            this.operation_error = Some(error.to_string().into());
                        }
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
                    .on_action(cx.listener(|this, _: &NewTab, _, cx| this.new_tab(cx)))
                    .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                        this.request_close_tab(this.active, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &NextTab, _, cx| this.next_tab(cx)))
                    .on_action(cx.listener(|this, _: &PrevTab, _, cx| this.prev_tab(cx)))
                    .on_action(cx.listener(|this, _: &CycleProfile, _, cx| {
                        // ⌘⇧P toggles the profile picker.
                        if !this.modal_open() {
                            this.picker_open = !this.picker_open;
                            cx.notify();
                        }
                    }))
                    .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                        this.menu_at = None;
                        cx.notify();
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
                                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
                        ),
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
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if operation_error_visible {
                                this.operation_error = None;
                            } else {
                                this.persistence_error = None;
                            }
                            cx.notify();
                        })),
                )
            })
            // Modal reviews remain the final children so no terminal surface
            // can paint over them or receive pointer input.
            .when_some(paste_confirmation, |terminal, alert| terminal.child(alert))
            .when_some(close_confirmation, |terminal, alert| terminal.child(alert))
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
    use alacritty_terminal::event::Event;

    #[derive(Clone, Default)]
    struct TitleEventProxy(Arc<Mutex<Vec<Option<String>>>>);

    impl EventListener for TitleEventProxy {
        fn send_event(&self, event: Event) {
            let title = match event {
                Event::Title(title) => Some(Some(title)),
                Event::ResetTitle => Some(None),
                _ => None,
            };
            if let (Some(title), Ok(mut events)) = (title, self.0.lock()) {
                events.push(title);
            }
        }
    }

    struct NoopHandler;

    impl vte::ansi::Handler for NoopHandler {}

    #[derive(Default)]
    struct ParserBoundRecorder {
        parameters: usize,
        intermediates: usize,
        ignored: bool,
        printed: String,
    }

    impl vte::Perform for ParserBoundRecorder {
        fn print(&mut self, character: char) {
            self.printed.push(character);
        }

        fn csi_dispatch(
            &mut self,
            parameters: &vte::Params,
            intermediates: &[u8],
            ignored: bool,
            _action: char,
        ) {
            self.parameters = parameters.iter().flatten().count();
            self.intermediates = intermediates.len();
            self.ignored = ignored;
        }
    }

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

    #[test]
    fn foreground_job_close_review_fails_closed() {
        assert!(!foreground_job_requires_confirmation(
            false,
            Some(40),
            Some(41)
        ));
        assert!(!foreground_job_requires_confirmation(
            true,
            Some(40),
            Some(40)
        ));
        assert!(foreground_job_requires_confirmation(
            true,
            Some(40),
            Some(41)
        ));
        assert!(foreground_job_requires_confirmation(true, None, Some(41)));
        assert!(foreground_job_requires_confirmation(true, Some(40), None));
    }

    #[test]
    fn child_exit_states_are_truthful_and_private_safe() {
        assert_eq!(SessionLifecycle::Running.status_message(), None);
        assert!(SessionLifecycle::WaitFailed.may_be_running());
        assert!(!SessionLifecycle::StartFailed.may_be_running());
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 0,
                signal: None
            }
            .status_message()
            .as_deref(),
            Some("The shell exited successfully.")
        );
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 7,
                signal: None
            }
            .status_message()
            .as_deref(),
            Some("The shell exited with status 7.")
        );
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 1,
                signal: Some("Hangup".into())
            }
            .status_message()
            .as_deref(),
            Some("The shell was terminated by Hangup.")
        );
        assert_eq!(
            lifecycle_after_wait(Ok(ExitStatus::with_exit_code(9))),
            SessionLifecycle::Exited {
                exit_code: 9,
                signal: None
            }
        );
        assert_eq!(
            lifecycle_after_wait(Ok(ExitStatus::with_signal("Hangup"))),
            SessionLifecycle::Exited {
                exit_code: 1,
                signal: Some("Hangup".into())
            }
        );
        assert_eq!(
            lifecycle_after_wait(Err(std::io::Error::other("private diagnostic"))),
            SessionLifecycle::WaitFailed
        );
    }

    #[test]
    fn terminal_resources_have_explicit_bounds() {
        assert_eq!(MAX_PASTE_BYTES, 1024 * 1024);
        assert_eq!(MAX_TABS, 16);
        assert_eq!(SESSION_WORKERS_PER_TAB, 2);
        assert_eq!(SESSION_WORKER_STACK_BYTES, 512 * 1024);
        assert_eq!(
            max_session_worker_stack_bytes_per_window(),
            16 * 1024 * 1024
        );
        assert_eq!(terminal_config(SCROLLBACK_LINES).scrolling_history, 10_000);
        assert_eq!(
            grid_dimensions(0.0, 0.0, CELL_W, LINE_H),
            TermSize {
                cols: MIN_COLS,
                lines: MIN_ROWS
            }
        );
        assert_eq!(
            grid_dimensions(f32::MAX, f32::MAX, CELL_W, LINE_H),
            TermSize {
                cols: MAX_COLS,
                lines: MAX_ROWS
            }
        );
        assert_eq!(
            grid_dimensions(f32::NAN, f32::NAN, CELL_W, LINE_H),
            TermSize {
                cols: MIN_COLS,
                lines: MIN_ROWS
            }
        );
    }

    #[test]
    fn vte_synchronized_update_buffer_stops_at_the_pinned_limit() {
        let mut parser: Processor = Processor::new();
        let mut handler = NoopHandler;
        parser.advance(&mut handler, b"\x1b[?2026h");

        let payload = vec![b'x'; MAX_VTE_SYNC_BUFFER_BYTES - 2];
        parser.advance(&mut handler, &payload);
        assert_eq!(parser.sync_bytes_count(), MAX_VTE_SYNC_BUFFER_BYTES - 2);

        parser.advance(&mut handler, b"x");
        assert_eq!(parser.sync_bytes_count(), 0);
    }

    #[test]
    fn vte_parser_arrays_and_partial_utf8_are_bounded() {
        let mut parser = vte::Parser::new();
        let mut recorder = ParserBoundRecorder::default();
        let mut parameters = b"\x1b[".to_vec();
        parameters.extend_from_slice("1;".repeat(MAX_VTE_CSI_PARAMETERS + 8).as_bytes());
        parameters.push(b'm');
        parser.advance(&mut recorder, &parameters);
        assert_eq!(recorder.parameters, MAX_VTE_CSI_PARAMETERS);
        assert!(recorder.ignored);

        recorder = ParserBoundRecorder::default();
        parser.advance(&mut recorder, b"\x1b[!!!m");
        assert_eq!(recorder.intermediates, MAX_VTE_INTERMEDIATES);
        assert!(recorder.ignored);

        recorder = ParserBoundRecorder::default();
        for byte in "😀".as_bytes().chunks(1) {
            parser.advance(&mut recorder, byte);
        }
        assert_eq!("😀".len(), MAX_UTF8_SCALAR_BYTES);
        assert_eq!(recorder.printed, "😀");
    }

    #[test]
    fn alacritty_title_stack_evicts_at_the_pinned_depth() {
        let proxy = TitleEventProxy::default();
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, proxy.clone());
        let mut parser: Processor = Processor::new();

        for title in 0..=MAX_TITLE_STACK_DEPTH {
            let sequence = format!("\x1b]0;{title}\x07\x1b[22t");
            parser.advance(&mut term, sequence.as_bytes());
        }
        if let Ok(mut events) = proxy.0.lock() {
            events.clear();
        }

        for _ in 0..=MAX_TITLE_STACK_DEPTH {
            parser.advance(&mut term, b"\x1b[23t");
        }

        let events = proxy
            .0
            .lock()
            .expect("title event lock should remain healthy");
        assert_eq!(events.len(), MAX_TITLE_STACK_DEPTH);
        assert_eq!(events.first().and_then(Option::as_deref), Some("4096"));
        assert_eq!(events.last().and_then(Option::as_deref), Some("1"));
    }

    #[test]
    fn aggregate_scrollback_budget_covers_every_supported_tab_count() {
        assert_eq!(scrollback_limit_for_tab_count(0), 0);
        assert_eq!(scrollback_limit_for_tab_count(1), SCROLLBACK_LINES);
        assert_eq!(scrollback_limit_for_tab_count(2), SCROLLBACK_LINES);
        assert!(bounded_cell_row_bytes() >= MAX_COLS * std::mem::size_of::<Cell>());
        assert!(
            retained_row_slots_bytes_per_tab()
                >= (SCROLLBACK_LINES + MAX_ROWS * VISIBLE_GRID_COPIES)
                    * std::mem::size_of::<Row<Cell>>()
        );

        let mut previous = SCROLLBACK_LINES;
        for tabs in 1..=MAX_TABS {
            let history = scrollback_limit_for_tab_count(tabs);
            assert!(history <= previous);
            assert!(
                bounded_grid_base_bytes(tabs, history) <= MAX_GRID_BASE_BYTES_PER_WINDOW,
                "{tabs} tabs with {history} history lines exceeded the base-grid budget"
            );
            previous = history;
        }

        let crowded_history = scrollback_limit_for_tab_count(MAX_TABS);
        assert!(crowded_history >= 650);
        assert!(crowded_history < SCROLLBACK_LINES);
        assert!(
            bounded_grid_base_bytes(MAX_TABS, crowded_history + 1) > MAX_GRID_BASE_BYTES_PER_WINDOW
        );
        assert_eq!(
            terminal_config(usize::MAX).scrolling_history,
            SCROLLBACK_LINES
        );
    }

    #[test]
    fn history_rebalance_preserves_zero_history_on_the_alternate_screen() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();

        parser.advance(&mut term, b"\x1b[?1049h");
        term.set_options(terminal_config(2));
        for _ in 0..20 {
            parser.advance(&mut term, b"alternate\r\n");
        }
        assert_eq!(term.grid().history_size(), 0);

        parser.advance(&mut term, b"\x1b[?1049l");
        for _ in 0..20 {
            parser.advance(&mut term, b"primary\r\n");
        }
        assert_eq!(term.grid().history_size(), 2);
    }

    #[test]
    fn output_filter_preserves_normal_unicode_and_split_valid_osc() {
        let chunks: &[&[u8]] = &[
            b"plain \xce",
            b"\xbb \x1b",
            b"]0;Private-safe title",
            b"\x1b",
            b"\\ tail \x1b[31mred",
        ];
        let mut filter = OutputFilter::default();
        let mut scratch = Vec::new();
        let mut output = Vec::new();
        for chunk in chunks {
            filter.filter_into(chunk, &mut scratch);
            output.extend_from_slice(&scratch);
        }

        assert_eq!(
            output,
            b"plain \xce\xbb \x1b]0;Private-safe title\x1b\\ tail \x1b[31mred"
        );
        assert_eq!(filter.buffered_bytes(), 0);
    }

    #[test]
    fn output_filter_drops_overlong_malformed_and_hyperlink_osc() {
        let mut filter = OutputFilter::default();
        let mut scratch = Vec::new();
        let mut output = Vec::new();

        let mut exact_prefix = b"before\x1b]0;".to_vec();
        exact_prefix.resize(
            exact_prefix.len() + MAX_OSC_PAYLOAD_BYTES.saturating_sub(2),
            b'a',
        );
        filter.filter_into(&exact_prefix, &mut scratch);
        output.extend_from_slice(&scratch);
        assert_eq!(output, b"before");
        assert_eq!(
            filter.buffered_bytes(),
            MAX_OSC_PAYLOAD_BYTES + b"\x1b]".len()
        );

        filter.filter_into(b"x\x07after", &mut scratch);
        output.extend_from_slice(&scratch);
        assert_eq!(output, b"beforeafter");
        assert_eq!(filter.buffered_bytes(), 0);

        filter.filter_into(b"\x1b]8;;https://example.invalid\x1b\\safe", &mut scratch);
        assert_eq!(scratch, b"safe");
        filter.filter_into(
            b"\x1b]\x008;;https://example.invalid\x07still-safe",
            &mut scratch,
        );
        assert_eq!(scratch, b"still-safe");

        filter.filter_into(b"\x1b]0;malformed\x1bXresumed", &mut scratch);
        assert_eq!(scratch, b"Xresumed");
        assert_eq!(filter.buffered_bytes(), 0);

        // C0 controls do not leave VTE's Escape state, so they must not allow
        // a split `ESC <control> ]` sequence to bypass the OSC limit.
        let mut bypass_filter = OutputFilter::default();
        bypass_filter.filter_into(b"\x1b\x07", &mut scratch);
        assert_eq!(scratch, b"\x07");
        let mut bypass = b"]0;".to_vec();
        bypass.resize(bypass.len() + MAX_OSC_PAYLOAD_BYTES + 1, b'b');
        bypass.extend_from_slice(b"\x07safe");
        bypass_filter.filter_into(&bypass, &mut scratch);
        assert_eq!(scratch, b"safe");
        assert_eq!(bypass_filter.buffered_bytes(), 0);
    }

    #[test]
    fn output_filter_accepts_the_exact_payload_limit_and_bel() {
        let mut sequence = b"\x1b]2;".to_vec();
        sequence.resize(
            sequence.len() + MAX_OSC_PAYLOAD_BYTES.saturating_sub(2),
            b't',
        );
        sequence.push(0x07);

        let mut filter = OutputFilter::default();
        let mut output = Vec::new();
        filter.filter_into(&sequence, &mut output);

        assert_eq!(output, sequence);
        assert_eq!(filter.buffered_bytes(), 0);
    }

    fn append_combining_marks(bytes: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            bytes.extend_from_slice("\u{301}".as_bytes());
        }
    }

    #[test]
    fn combining_marks_are_bounded_per_exact_cell_without_losing_style() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        let mut bytes = b"\x1b[31;4ma".to_vec();
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);
        bytes.push(b'b');
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);

        advance_filtered_output(&mut parser, &mut term, &bytes);

        for column in [Column(0), Column(1)] {
            let cell = &term.grid()[Line(0)][column];
            assert_eq!(
                cell.zerowidth().map(<[char]>::len),
                Some(MAX_COMBINING_MARKS_PER_CELL)
            );
            assert_eq!(cell.fg, Color::Named(NamedColor::Red));
            assert!(cell.flags.contains(Flags::UNDERLINE));
        }

        // CSI REP is entirely ASCII but can repeat the preceding zero-width
        // scalar many times; its final `b` is an explicit cap boundary.
        advance_filtered_output(&mut parser, &mut term, b"\x1b[200b");
        assert_eq!(
            term.grid()[Line(0)][Column(1)]
                .zerowidth()
                .map(<[char]>::len),
            Some(MAX_COMBINING_MARKS_PER_CELL)
        );
    }

    #[test]
    fn combining_cap_handles_wide_cells_and_split_utf8() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        let mut bytes = "界".as_bytes().to_vec();
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);
        let split = "界".len() + 1;

        advance_filtered_output(&mut parser, &mut term, &bytes[..split]);
        advance_filtered_output(&mut parser, &mut term, &bytes[split..]);

        let wide = &term.grid()[Line(0)][Column(0)];
        assert_eq!(
            wide.zerowidth().map(<[char]>::len),
            Some(MAX_COMBINING_MARKS_PER_CELL)
        );
        assert!(term.grid()[Line(0)][Column(1)]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER));
    }

    #[test]
    fn paste_line_count_normalizes_platform_boundaries() {
        assert_eq!(logical_line_count(""), 0);
        assert_eq!(logical_line_count("one"), 1);
        assert_eq!(logical_line_count("one\ntwo"), 2);
        assert_eq!(logical_line_count("one\r\ntwo"), 2);
        assert_eq!(logical_line_count("one\rtwo"), 2);
        assert_eq!(logical_line_count("one\r\ntwo\nthree\rfour"), 4);
    }

    #[test]
    fn bracketed_paste_cannot_embed_its_terminator() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(SCROLLBACK_LINES), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[?2004h");
        assert!(term.mode().contains(TermMode::BRACKETED_PASTE));
        parser.advance(&mut term, b"\x1b[?2004l");
        assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));

        let payload = prepare_paste("one\x1b[201~two\x03\nthree", true);
        let mut expected = BRACKETED_PASTE_START.to_vec();
        expected.extend_from_slice(b"one[201~two\nthree");
        expected.extend_from_slice(BRACKETED_PASTE_END);

        assert_eq!(payload, expected);
        assert_eq!(
            payload
                .windows(BRACKETED_PASTE_END.len())
                .filter(|window| *window == BRACKETED_PASTE_END)
                .count(),
            1
        );
    }

    #[test]
    fn unbracketed_paste_uses_return_and_rejects_controls() {
        assert_eq!(
            prepare_paste("one\r\ntwo\nthree\rfour", false),
            b"one\rtwo\rthree\rfour"
        );
        assert!(has_unsafe_unbracketed_control("one\x1btwo"));
        assert!(has_unsafe_unbracketed_control("one\x03two"));
        assert!(!has_unsafe_unbracketed_control("one\ttwo\nthree"));
    }

    #[test]
    fn pending_paste_debug_redacts_clipboard_text() {
        let pending = PendingPaste {
            session_id: 7,
            text: "private clipboard body".into(),
            line_count: 2,
            byte_count: 22,
        };
        let debug = format!("{pending:?}");

        assert!(!debug.contains("private clipboard body"));
        assert!(debug.contains("<private>"));
        assert!(debug.contains("line_count: 2"));
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
    fn navigation_and_function_keys_encode_xterm_modifiers() {
        let alt = gpui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift_control = gpui::Modifiers {
            shift: true,
            control: true,
            ..Default::default()
        };

        assert_eq!(
            encode_key(&test_keystroke("left", None, alt), TermMode::APP_CURSOR),
            b"\x1b[1;3D"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("delete", None, shift_control),
                TermMode::default()
            ),
            b"\x1b[3;6~"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("f1", None, gpui::Modifiers::default()),
                TermMode::default()
            ),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("f1", None, shift_control),
                TermMode::default()
            ),
            b"\x1b[1;6P"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("f5", None, shift_control),
                TermMode::default()
            ),
            b"\x1b[15;6~"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("f20", None, gpui::Modifiers::default()),
                TermMode::default()
            ),
            b"\x1b[34~"
        );
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

    #[test]
    fn text_control_and_meta_input_remain_exact() {
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        let alt = gpui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift = gpui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let platform = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };

        assert_eq!(
            encode_key(&test_keystroke("c", None, control), TermMode::default()),
            b"\x03"
        );
        assert_eq!(
            encode_key(&test_keystroke("[", None, control), TermMode::default()),
            b"\x1b"
        );
        assert_eq!(
            encode_key(&test_keystroke("space", None, control), TermMode::default()),
            b"\x00"
        );
        assert_eq!(
            encode_key(
                &test_keystroke("f", Some("f"), gpui::Modifiers::default()),
                TermMode::default()
            ),
            b"f"
        );
        assert_eq!(
            encode_key(&test_keystroke("x", Some("λ"), alt), TermMode::default()),
            "\u{1b}λ".as_bytes()
        );
        assert_eq!(
            encode_key(
                &test_keystroke("tab", Some("\t"), shift),
                TermMode::default()
            ),
            b"\x1b[Z"
        );
        assert!(encode_key(&test_keystroke("x", None, platform), TermMode::default()).is_empty());
    }
}
