//! rmac Terminal — a fast, native terminal emulator.

mod controller;
mod emulator;
mod ime;
mod keyboard;
mod mouse;
mod output_filter;
mod paste;
mod profiles;
mod session;
mod storage;

fn main() {
    rmac_ui::boot_app(
        rmac_ui::app_id::TERMINAL,
        "Terminal",
        820.0,
        560.0,
        controller::TerminalView::new,
    );
}
