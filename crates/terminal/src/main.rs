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
mod session_restore;
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

    // Application ▸ Quit and Keep Windows (TERM-22): a plain launch (no
    // `-e`, no explicit profile/arguments of its own) reopens whatever was
    // kept last time, once — read and removed here, before GPUI starts, so
    // a crashed read never reopens the same session twice. Any argument at
    // all (a file manager's "Open Terminal here", `-e`, …) means the user
    // asked for something specific instead, so the kept session is left
    // for the next plain launch rather than silently dropped.
    let windows = if arguments.is_empty() {
        kept_window_launch_arguments().unwrap_or_else(|| vec![arguments])
    } else {
        vec![arguments]
    };

    // Several Terminal windows is the default way of working (⌘N). One
    // process owns the app's menu and every window; the app stays running,
    // in the Dock with its menu, after the last one closes — as on the Mac.
    rmac_ui::boot_app_instance(
        rmac_ui::app_id::TERMINAL,
        "Terminal",
        // 80 × 24 cells of 7 × 14 plus the measured insets and title bar.
        580.0,
        385.0,
        windows,
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
            // Application ▸ Quit and Keep Windows (TERM-22): a relaunch
            // carrying a saved window takes over tab/cwd/scrollback setup
            // entirely, ignoring `profile`/`exec` above.
            let restore = cli::parse_restore_flag(arguments);
            controller::TerminalView::new(window, cx, profile, exec, restore)
        },
        controller::register_windowless_actions,
    );
}

/// Application ▸ Quit and Keep Windows (TERM-22): one `--restore=` argument
/// list per window last kept, or `None` if there is nothing kept (the
/// common case — an ordinary launch falls back to the default window).
/// Reading and removing the file happens together so a session is never
/// replayed twice.
fn kept_window_launch_arguments() -> Option<Vec<Vec<String>>> {
    let path = session_restore::kept_windows_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let windows: Vec<session_restore::RestoreWindow> = serde_json::from_slice(&bytes).ok()?;
    let arguments: Vec<Vec<String>> = windows
        .iter()
        .filter(|window| !window.is_empty())
        .filter_map(|window| cli::restore_flag(window))
        .map(|flag| vec![flag])
        .collect();
    (!arguments.is_empty()).then_some(arguments)
}
