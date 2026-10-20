//! `rmac-night-shift --policy FILE`: runs `wlsunset` for Settings ▸
//! Displays ▸ Night Shift (`rmac_shortcuts::night_shift`).

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [flag, policy] = arguments.as_slice() else {
        eprintln!("usage: rmac-night-shift --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    };
    if flag != "--policy" {
        eprintln!("usage: rmac-night-shift --policy ABSOLUTE_PATH");
        return ExitCode::FAILURE;
    }
    match rmac_shortcuts::night_shift::supervise(Path::new(policy)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
