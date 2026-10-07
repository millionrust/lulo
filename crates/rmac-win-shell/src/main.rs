//! `lulo-shell`: the Lulo layer's menu bar, Dock and Spotlight on Windows.
//! Started by `lulo-session`, which restores the Windows desktop whenever
//! this process stops (ADR 0023 phase 3).

// A GUI process on Windows: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    std::process::exit(rmac_win_shell::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("lulo-shell is the Lulo layer for Windows; Lulo OS runs its own shell.");
    std::process::exit(2);
}
