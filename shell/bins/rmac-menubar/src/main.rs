#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    rmac_shell_menubar::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("top-bar requires Linux and: cargo run --features wayland --bin top-bar");
    std::process::exit(2);
}
