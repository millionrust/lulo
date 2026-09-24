//! Fuzzes the VTE/ANSI escape-sequence parser that crates/terminal feeds
//! raw PTY output through (crates/terminal/src/emulator.rs,
//! `advance_filtered_output`, wraps `vte::ansi::Processor::advance` over
//! `alacritty_terminal::term::Term`).
//!
//! `crates/terminal` only has a `[[bin]]` target today, so
//! `advance_filtered_output` itself is not reachable from an external fuzz
//! crate without adding a library target to that crate — a restructuring
//! this task intentionally avoids (see docs/ci.md). This harness instead
//! drives the same pinned `vte`/`alacritty_terminal` versions the terminal
//! depends on (see Cargo.toml) directly, which covers the escape-sequence
//! parser itself; it does not cover rmac's thin combining-mark-capping
//! wrapper around it.
#![no_main]

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use libfuzzer_sys::fuzz_target;
use vte::ansi::Processor;

/// A fixed 80x24 viewport with no scrollback: only escape-sequence handling
/// is under test here, not grid resizing.
struct FixedDimensions;

impl Dimensions for FixedDimensions {
    fn total_lines(&self) -> usize {
        24
    }

    fn screen_lines(&self) -> usize {
        24
    }

    fn columns(&self) -> usize {
        80
    }
}

fuzz_target!(|data: &[u8]| {
    let mut parser = Processor::new();
    let mut term = Term::new(Config::default(), &FixedDimensions, VoidListener);
    parser.advance(&mut term, data);
});
