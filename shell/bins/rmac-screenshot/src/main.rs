//! `screenshot`: Lulo OS's screenshot service and its commands
//! (`rmac_shell_screenshot::service`).

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    if let Err(error) = rmac_shell_screenshot::service::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!(
        "Screenshots require Linux and: cargo run --features wayland --bin screenshot -- --service"
    );
    std::process::exit(2);
}
