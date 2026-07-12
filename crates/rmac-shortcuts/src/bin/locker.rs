use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [flag, config] = arguments.as_slice() else {
        eprintln!("usage: rmac-locker --config ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    };
    if flag != "--config" {
        eprintln!("usage: rmac-locker --config ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    }
    match rmac_shortcuts::lock::supervise(Path::new(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
