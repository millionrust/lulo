use std::process::ExitCode;

fn main() -> ExitCode {
    match async_io::block_on(rmac_shortcuts::lock::coordinate()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
