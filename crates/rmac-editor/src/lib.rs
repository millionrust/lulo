//! `rmac-editor` — the shared text-editing core for rmac apps.
//!
//! Wraps gpui-component's rope-backed `InputState` with rmac's standard
//! configuration so the Text Editor and Notes (and anything else with a text
//! body) behave and look identical. Re-exports the `Input` element for rendering.

use gpui::{AppContext as _, Context, Entity, Window};

pub use gpui_component::input::{Input, InputState};

/// Create a standard multi-line, soft-wrapped editor state.
///
/// Render it with [`Input::new`]`(&state).h_full().appearance(false)`.
pub fn multiline<T: 'static>(
    placeholder: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| {
        InputState::new(window, cx)
            .multi_line(true)
            .soft_wrap(true)
            .placeholder(placeholder)
    })
}

/// Read the current text out of an editor state.
pub fn value<T: 'static>(state: &Entity<InputState>, cx: &Context<T>) -> String {
    state.read(cx).value().to_string()
}

/// Derive a short title from note/body text: first non-empty line, trimmed.
pub fn title_from_body(body: &str, fallback: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let t: String = l.trim_start_matches('#').trim().chars().take(60).collect();
            if t.is_empty() {
                fallback.to_string()
            } else {
                t
            }
        })
        .unwrap_or_else(|| fallback.to_string())
}
