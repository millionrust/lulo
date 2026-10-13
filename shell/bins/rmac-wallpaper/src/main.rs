#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    rmac_shell_wallpaper::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("wallpaper requires Linux and: cargo run --features wayland --bin wallpaper");
    std::process::exit(2);
}
