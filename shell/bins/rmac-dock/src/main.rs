#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    rmac_shell_dock::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("Dock requires Linux and: cargo run --features wayland --bin dock");
    std::process::exit(2);
}
