// A GUI app on Windows: no console window behind it (ADR 0023).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// One System Settings on every platform: the same panes and views, each
// backed by its platform's service crate (Linux services on Lulo OS, the
// Windows APIs on Windows). Panes a platform cannot really drive are left
// out of its sidebar (`navigation::platform_supports`), and hardware panes
// follow the hardware scan (`hardware`).
mod appearance;
mod connectivity;
mod controller;
mod displays;
mod focus;
mod hardware;
mod input;
mod navigation;
mod notifications;
mod power;
mod responsive_layout;
mod service_updates;
mod settings_search;
mod shell_settings;
mod sound;
mod storage_categories;
mod system_environment;

fn main() {
    controller::run();
}
