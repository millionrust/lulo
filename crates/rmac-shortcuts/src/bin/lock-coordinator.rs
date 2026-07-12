use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [flag, policy] = arguments.as_slice() else {
        eprintln!("usage: rmac-lock-coordinator --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    };
    if flag != "--policy" {
        eprintln!("usage: rmac-lock-coordinator --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    }
    match async_io::block_on(rmac_shortcuts::lock::coordinate(Path::new(policy))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
