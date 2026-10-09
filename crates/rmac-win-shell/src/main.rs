//! `lulo-shell`: the Lulo layer's menu bar, Dock and Spotlight on Windows.
//! Started by `lulo-session`, which restores the Windows desktop whenever
//! this process stops (ADR 0023 phase 3).

// A GUI process on Windows: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    // The icon helper (`win::icons`): reads Windows icons for the shell in
    // a process of its own, so the shell libraries never load into it.
    // The Apps folder helper (`win::catalog`), for the same reason.
    match std::env::args().nth(1).as_deref() {
        Some(rmac_win_shell::win::icons::HELPER_SWITCH) => {
            std::process::exit(rmac_win_shell::win::icons::run_helper())
        }
        Some(rmac_win_shell::win::catalog::APPS_HELPER_SWITCH) => {
            std::process::exit(rmac_win_shell::win::catalog::run_apps_helper())
        }
        // One icon written as a PNG (`win::shell_icons`), for the same
        // reason.
        // Brightness through WMI (`rmac_osd::windows`), for the same reason.
        Some("--brightness") => std::process::exit(rmac_osd::windows::run_brightness_helper(
            &std::env::args().skip(2).collect::<Vec<_>>(),
        )),
        Some(rmac_win_shell::win::shell_icons::SAVE_SWITCH) => {
            std::process::exit(rmac_win_shell::win::shell_icons::run_save())
        }
        _ => {}
    }
    std::process::exit(rmac_win_shell::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("lulo-shell is the Lulo layer for Windows; Lulo OS runs its own shell.");
    std::process::exit(2);
}
