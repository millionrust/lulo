//! rmac ⌘Tab application switcher (docs/decisions/0009-app-switcher.md).
//! The service and its panel are the library (`rmac_shell_app_switcher`),
//! which `lulo-shell` also hosts on Windows.

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    if let Err(error) = rmac_shell_app_switcher::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("The app switcher requires Linux and: cargo run --features wayland --bin app-switcher -- --service");
    std::process::exit(2);
}
