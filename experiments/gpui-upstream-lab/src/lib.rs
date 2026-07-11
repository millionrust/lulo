use std::{env, fs, time::Duration};

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

/// Delay to the next wall-clock minute without a periodic redraw loop.
pub fn delay_until_next_minute(epoch_millis: u128) -> Duration {
    const MINUTE_MILLIS: u128 = 60_000;
    let remaining = MINUTE_MILLIS - epoch_millis % MINUTE_MILLIS;
    Duration::from_millis(remaining as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_delay_is_bounded_and_never_busy_loops() {
        assert_eq!(delay_until_next_minute(0), Duration::from_secs(60));
        assert_eq!(delay_until_next_minute(59_999), Duration::from_millis(1));
        assert_eq!(delay_until_next_minute(60_000), Duration::from_secs(60));
    }
}
