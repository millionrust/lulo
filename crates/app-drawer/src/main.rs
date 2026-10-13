//! rmac Apps — a searchable grid of installed applications.

use rmac_app_drawer::{run_mode, RunMode};

fn main() {
    match run_mode(std::env::args().skip(1)) {
        RunMode::Service { show_on_start } => rmac_app_drawer::run_service(show_on_start),
        RunMode::Standalone => rmac_app_drawer::run_standalone(),
    }
}
