//! `rmac-polkit-agent`: Lulo's polkit authentication agent (SWU-07), a user
//! service of rmac-session.target.

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = rmac_polkit_agent::ui::run() {
        eprintln!("rmac-polkit-agent: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("rmac-polkit-agent needs Linux and polkit");
    std::process::exit(2);
}
