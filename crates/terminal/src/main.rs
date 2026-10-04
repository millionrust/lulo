//! rmac Terminal — a fast, native terminal emulator.

mod cli;
mod controller;
mod emulator;
mod find;
mod hyperlink;
mod ime;
mod job;
mod keyboard;
mod mouse;
mod output_filter;
mod paste;
mod profiles;
mod session;
mod settings;
mod settings_window;
mod shell_integration;
mod storage;
mod title;
mod ui_state;
mod working_directory;

fn main() {
    // `-e PROGRAM ARGS…` (`man x-terminal-emulator`): exec PROGRAM directly
    // instead of a shell. Checked against this process's own argv before
    // GPUI starts, so a bad invocation (`-e` with nothing after it) exits
    // with a usage message rather than opening a window.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = cli::parse_exec_flag(&arguments) {
        eprintln!("rmac-terminal: {error}");
        std::process::exit(1);
    }

    // Several Terminal windows is the default way of working (⌘N). One
    // process owns the app's menu and every window; the app stays running,
    // in the Dock with its menu, after the last one closes — as on the Mac.
    rmac_ui::boot_app_instance(
        rmac_ui::app_id::TERMINAL,
        "Terminal",
        // 80 × 24 cells of 7 × 14 plus the measured insets and title bar.
        580.0,
        385.0,
        vec![arguments],
        |arguments, window, cx| {
            let profile = arguments
                .iter()
                .find_map(|argument| argument.strip_prefix("--profile="))
                .and_then(|index| index.parse::<usize>().ok())
                .filter(|index| *index < profiles::PROFILES.len());
            // A bad `-e` was already rejected above for this process's own
            // launch; a second launch handed off over D-Bus is re-checked
            // the same way and just falls back to a shell if it recurs.
            let exec = cli::parse_exec_flag(arguments).ok().flatten();
            controller::TerminalView::new(window, cx, profile, exec)
        },
        controller::register_windowless_actions,
    );
}
