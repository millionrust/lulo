//! rmac App Drawer — a searchable grid of installed applications.

mod catalog;
mod service;
mod view;

use rmac_app_drawer::{run_mode, RunMode};

use view::AppDrawer;

gpui::actions!(
    app_drawer,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        Launch,
        ClearSearch,
        OpenApp,
        RevealInFinder
    ]
);

fn main() {
    match run_mode(std::env::args().skip(1)) {
        RunMode::Service { show_on_start } => service::run(show_on_start),
        RunMode::Standalone => rmac_ui::boot_app(
            rmac_ui::app_id::APP_DRAWER,
            "Applications",
            1080.0,
            720.0,
            |window, cx| {
                cx.bind_keys(service::key_bindings());
                AppDrawer::new(None, window, cx)
            },
        ),
    }
}
