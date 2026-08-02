//! rmac Terminal — a fast, native terminal emulator.

mod controller;
mod emulator;
mod hyperlink;
mod ime;
mod job;
mod keyboard;
mod mouse;
mod output_filter;
mod paste;
mod profiles;
mod session;
mod shell_integration;
mod storage;
mod title;
mod ui_state;
mod working_directory;

fn main() {
    rmac_ui::boot_app(
        rmac_ui::app_id::TERMINAL,
        "Terminal",
        820.0,
        560.0,
        controller::TerminalView::new,
    );
}
