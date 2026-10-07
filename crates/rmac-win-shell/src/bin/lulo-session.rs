//! `lulo-session`: turns the Lulo layer on over the Windows desktop, and
//! gives the desktop back whenever it stops (ADR 0023 phase 3).

// A background process on Windows: no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    std::process::exit(rmac_win_shell::win::session::run(&arguments));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("lulo-session is the Lulo layer for Windows; Lulo OS runs its own session.");
    std::process::exit(2);
}
