// A GUI app on Windows: no console window behind it (ADR 0023).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Lulo OS's System Settings (`controller`) and the Linux service panes it
// drives are `cfg(unix)`: Linux, plus macOS developer builds. Windows has
// its own small Settings (`win`, ADR 0023 phase 2e) that shares the
// platform-neutral modules below and reaches Windows through one facade
// (`win::host`).
#[cfg_attr(windows, allow(dead_code))]
mod appearance;
#[cfg(unix)]
mod connectivity;
#[cfg(unix)]
mod controller;
#[cfg(unix)]
mod displays;
#[cfg(unix)]
mod focus;
#[cfg(unix)]
mod hardware;
#[cfg(unix)]
mod input;
#[cfg_attr(windows, allow(dead_code))]
mod navigation;
#[cfg(unix)]
mod notifications;
#[cfg(unix)]
mod power;
#[cfg(unix)]
mod responsive_layout;
#[cfg(unix)]
mod service_updates;
#[cfg_attr(windows, allow(dead_code))]
mod settings_search;
#[cfg_attr(windows, allow(dead_code))]
mod shell_settings;
#[cfg(unix)]
mod sound;
#[cfg(unix)]
mod storage_categories;
#[cfg(unix)]
mod system_environment;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(unix)]
    controller::run();
    #[cfg(windows)]
    win::run();
}
