//! rmac (ADR 0026): an opt-out for the resize change in `Window::bounds_changed`.
//!
//! `RMAC_GPUI_RESIZE_REFRESH=1` brings back upstream's full refresh on every
//! bounds change, so the same build can be measured both ways
//! (`scripts/behavior/run_speed_sweep.py`). Read once per process.

use std::sync::OnceLock;

pub(crate) fn full_refresh_on_resize() -> bool {
    static FULL_REFRESH: OnceLock<bool> = OnceLock::new();
    *FULL_REFRESH.get_or_init(|| {
        std::env::var_os("RMAC_GPUI_RESIZE_REFRESH").is_some_and(|value| value == "1")
    })
}
