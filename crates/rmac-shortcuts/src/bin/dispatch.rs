use std::path::Path;
use std::process::ExitCode;

use rmac_shortcuts::{dispatch, write_niri_fallback, ShortcutId};

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [command, output, dispatcher] if command == "write-niri-fallback" => {
            write_niri_fallback(Path::new(output), Path::new(dispatcher))
        }
        [id] => dispatch(&ShortcutId(id.clone())),
        _ => {
            eprintln!(
                "usage: rmac-shortcut-dispatch SHORTCUT_ID | write-niri-fallback OUTPUT DISPATCHER"
            );
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
