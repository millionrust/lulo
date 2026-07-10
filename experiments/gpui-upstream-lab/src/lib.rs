use std::{env, fs};

use gpui::Window;

const READY_FILE_ENV: &str = "RMAC_SMOKE_READY_FILE";

/// Writes an opt-in marker after GPUI finishes the window's first frame.
///
/// The nested-Wayland smoke harness uses this to distinguish a rendered
/// surface from a process that merely reached `open_window`. Normal runs do
/// not set the environment variable and perform no filesystem I/O.
pub fn mark_first_frame(window: &Window, probe: &'static str) {
    let Some(path) = env::var_os(READY_FILE_ENV) else {
        return;
    };

    window.on_next_frame(move |_, _| {
        fs::write(&path, format!("{probe}\n"))
            .unwrap_or_else(|error| panic!("write first-frame marker {path:?}: {error}"));
    });
}
