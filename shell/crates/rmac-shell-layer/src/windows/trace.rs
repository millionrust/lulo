//! What the Windows shell does, on stderr, for the Windows CI checks
//! (`scripts/windows/shell_smoke.py`): on with `LULO_SHELL_TRACE=1`, and a
//! single cached flag test otherwise. The shared views report where they
//! drew what a check clicks (Dock tiles, bar titles, desktop icons) in
//! physical screen pixels, once per change.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Whether `LULO_SHELL_TRACE=1`.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LULO_SHELL_TRACE").is_some_and(|value| value == "1"))
}

/// Say `message` (each of its lines) when tracing is on.
pub fn trace(message: impl FnOnce() -> String) {
    if enabled() {
        for line in message().lines() {
            eprintln!("lulo-shell: {line}");
        }
    }
}

thread_local! {
    static LAST: RefCell<HashMap<&'static str, String>> = RefCell::new(HashMap::new());
}

/// Say `message` under `key` only when it differs from the last one said
/// under that key, so a view can report its layout from every frame.
pub fn trace_changed(key: &'static str, message: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let message = message();
    let changed = LAST.with(|last| {
        let mut last = last.borrow_mut();
        if last.get(key) == Some(&message) {
            false
        } else {
            last.insert(key, message.clone());
            true
        }
    });
    if changed {
        trace(|| message);
    }
}

/// A point in `window`'s own logical coordinates as physical screen pixels.
pub fn screen_point(window: &gpui::Window, x: f32, y: f32) -> (i32, i32) {
    let origin = window.bounds().origin;
    let scale = window.scale_factor();
    (
        ((origin.x.as_f32() + x) * scale).round() as i32,
        ((origin.y.as_f32() + y) * scale).round() as i32,
    )
}
