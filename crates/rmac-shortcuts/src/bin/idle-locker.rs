use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [flag, policy] = arguments.as_slice() else {
        eprintln!("usage: rmac-idle-locker --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    };
    if flag != "--policy" {
        eprintln!("usage: rmac-idle-locker --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    }
    match rmac_shortcuts::lock::supervise_idle(Path::new(policy)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
