//! rmac Terminal — a fast, native terminal emulator.
//!
//! The Zed stack: `alacritty_terminal` drives the grid/escape-sequence state,
//! `portable-pty` runs the user's shell, and GPUI renders the cell grid. A
//! background thread reads PTY output and feeds the parser; the view renders
//! the grid each frame and writes keystrokes back to the PTY.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config, Term};
use gpui::{
    div, px, Context, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement,
    Render, Styled, Window,
};
use gpui_component::StyledExt as _;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use vte::ansi::{Color, NamedColor, Processor};

const COLS: usize = 100;
const ROWS: usize = 28;
const FONT: &str = "Menlo"; // macOS Terminal's default monospace
const FONT_SIZE: f32 = 13.0;
const LINE_H: f32 = 17.0;

// One Dark-ish palette.
const FG: u32 = 0xd4d4d4;
const BG: u32 = 0x1e1e1e;

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

/// No-op event listener — we poll the grid on a render timer instead.
#[derive(Clone)]
struct EventProxy;
impl EventListener for EventProxy {
    fn send_event(&self, _: Event) {}
}

struct TerminalView {
    term: Arc<Mutex<Term<EventProxy>>>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn MasterPty + Send>,
    focus: gpui::FocusHandle,
}

impl TerminalView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let size = TermSize {
            cols: COLS,
            lines: ROWS,
        };

        // Spawn the shell in a PTY.
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: ROWS as u16,
                cols: COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        let _child = pair.slave.spawn_command(cmd).expect("spawn shell");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer = pair.master.take_writer().expect("writer");

        let term = Arc::new(Mutex::new(Term::new(Config::default(), &size, EventProxy)));

        // Reader thread: feed PTY bytes into the terminal model.
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
                    }
                }
            }
        });

        let focus = cx.focus_handle();
        window.focus(&focus);

        // Redraw timer — keeps the view in sync with the model.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(33))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();

        Self {
            term,
            writer,
            _master: pair.master,
            focus,
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
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
            let _ = self.writer.write_all(&bytes);
            let _ = self.writer.flush();
        }
    }

    /// Build the visible grid as one styled line per row (color runs).
    fn render_rows(&self) -> Vec<gpui::AnyElement> {
        let Ok(term) = self.term.lock() else {
            return Vec::new();
        };
        let grid = term.grid();
        let cursor = grid.cursor.point;
        let cursor_line = cursor.line.0;
        let cursor_col = cursor.column.0;

        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(ROWS);
        for line in 0..ROWS as i32 {
            let row = &grid[Line(line)];
            let mut spans: Vec<gpui::AnyElement> = Vec::new();
            let mut run = String::new();
            let mut run_fg = conv(Color::Named(NamedColor::Foreground));
            let mut run_bg = conv(Color::Named(NamedColor::Background));

            for col in 0..COLS {
                let cell = &row[Column(col)];
                let mut fg = conv(cell.fg);
                let mut bg = conv(cell.bg);
                // Block cursor: invert the cell under the cursor.
                if line == cursor_line && col == cursor_col {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let ch = if cell.c == '\0' { ' ' } else { cell.c };

                if run.is_empty() {
                    run_fg = fg;
                    run_bg = bg;
                } else if fg != run_fg || bg != run_bg {
                    spans.push(span(&run, run_fg, run_bg));
                    run.clear();
                    run_fg = fg;
                    run_bg = bg;
                }
                run.push(ch);
            }
            if !run.is_empty() {
                spans.push(span(&run, run_fg, run_bg));
            }

            rows.push(div().flex().h(px(LINE_H)).children(spans).into_any_element());
        }
        rows
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.render_rows();
        div()
            .size_full()
            .v_flex()
            .bg(hsla(BG))
            .child(rmac_ui::toolbar(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .child("Terminal"),
            ))
            .child(
                div()
                    .track_focus(&self.focus)
                    .key_context("Terminal")
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                        this.on_key(ev);
                        cx.notify();
                    }))
                    .flex_1()
                    .p_2()
                    .bg(hsla(BG))
                    .font_family(FONT)
                    .text_size(px(FONT_SIZE))
                    .v_flex()
                    .children(rows),
            )
    }
}

fn span(text: &str, fg: Hsla, bg: Hsla) -> gpui::AnyElement {
    div()
        .text_color(fg)
        .bg(bg)
        .child(text.to_string())
        .into_any_element()
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
    match n {
        Background => split(BG),
        Foreground | Cursor => split(FG),
        Black => (0x28, 0x2c, 0x34),
        Red | BrightRed => (0xe0, 0x6c, 0x75),
        Green | BrightGreen => (0x98, 0xc3, 0x79),
        Yellow | BrightYellow => (0xe5, 0xc0, 0x7b),
        Blue | BrightBlue => (0x61, 0xaf, 0xef),
        Magenta | BrightMagenta => (0xc6, 0x78, 0xdd),
        Cyan | BrightCyan => (0x56, 0xb6, 0xc2),
        White => (0xab, 0xb2, 0xbf),
        BrightBlack => (0x5c, 0x63, 0x70),
        BrightWhite => (0xff, 0xff, 0xff),
        _ => split(FG),
    }
}

fn indexed(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => named(match i {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        }),
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
