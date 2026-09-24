//! rmac Terminal — a fast, native terminal emulator.

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
mod settings_window;
mod shell_integration;
mod storage;
mod title;
mod ui_state;
mod working_directory;

fn main() {
    // Several Terminal windows is the default way of working (⌘N). One
    // process owns the app's menu and every window; the app stays running,
    // in the Dock with its menu, after the last one closes — as on the Mac.
    rmac_ui::boot_app_instance(
        rmac_ui::app_id::TERMINAL,
        "Terminal",
        // 80 × 24 cells of 7 × 14 plus the measured insets and title bar.
        580.0,
        385.0,
        vec![Vec::new()],
        |_arguments, window, cx| controller::TerminalView::new(window, cx),
    );
}
