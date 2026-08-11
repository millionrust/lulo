//! rmac App Drawer — a searchable grid of installed applications.

mod catalog;
mod service;
mod view;

use rmac_app_drawer::{run_mode, RunMode};

use view::{AppDrawer, DRAWER_HEIGHT, DRAWER_WIDTH};

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
        RunMode::Standalone => {
            #[cfg(target_os = "linux")]
            {
                if let Err(error) =
                    rmac_shortcuts::dispatch(&rmac_shortcuts::ShortcutId("app-drawer".into()))
                {
                    eprintln!("Could not open Apps in Spotlight: {error}");
                    std::process::exit(1);
                }
            }
            #[cfg(not(target_os = "linux"))]
            rmac_ui::boot_app(
                rmac_ui::app_id::APP_DRAWER,
                "Apps",
                DRAWER_WIDTH,
                DRAWER_HEIGHT,
                |window, cx| {
                    cx.bind_keys(service::key_bindings());
                    AppDrawer::new(None, window, cx)
                },
            );
        }
    }
}
