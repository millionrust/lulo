//! `mission-control`: Lulo OS's resident Mission Control service and its
//! one-word command client (the library's `run`). On Windows the same
//! service runs inside `lulo-shell`.

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    if let Err(error) = rmac_shell_mission_control::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!(
        "Mission Control requires Linux and: cargo run --features wayland --bin mission-control -- --service"
    );
    std::process::exit(2);
}
