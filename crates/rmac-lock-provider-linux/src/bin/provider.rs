use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    match rmac_lock_provider_linux::development_process::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("the rmac lock provider is available only on Linux");
    ExitCode::FAILURE
}
