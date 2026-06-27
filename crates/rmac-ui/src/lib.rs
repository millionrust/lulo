//! `rmac-ui` — the shared design system for the rmac desktop suite.
//!
//! Every rmac app depends on this crate so they share one look: macOS-style
//! window chrome (traffic lights), a common theme, fonts, and a `boot` helper
//! that removes the per-app GPUI/Window/Root boilerplate.
//!
//! Apps render `rmac_ui::title_bar(...)` at the top of their view and call
//! `rmac_ui::boot(...)` from `main()`.

use gpui::{
    div, point, px, size, App, AppContext as _, Application, Bounds, Context, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use gpui_component::{Root, TitleBar};

// Re-exports so apps depend on one crate for theming; these also bring the
// traits into scope here for `.v_flex()`, `cx.theme()`, etc.
pub use gpui_component::{ActiveTheme, StyledExt};

/// Preferred UI font. Substitute for SF Pro — never ship Apple fonts.
/// Falls back to the platform default if not installed (font embedding lands later).
pub const UI_FONT: &str = "Inter";
/// Preferred monospace font (Terminal, Text Editor, code).
pub const MONO_FONT: &str = "JetBrains Mono";

/// Standard window options for an rmac app window: macOS traffic lights in the
/// canonical position, transparent titlebar so our chrome draws through.
pub fn window_options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(200.0), px(120.0)),
            size(px(width), px(height)),
        ))),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.0), px(12.0))),
        }),
        ..Default::default()
    }
}

/// The shared title bar: traffic-light gutter on the left, centered title.
/// Apps put this at the top of their root `div`.
pub fn title_bar(title: impl Into<SharedString>) -> impl IntoElement {
    let title: SharedString = title.into();
    TitleBar::new().child(
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .child(title),
    )
}

/// Boot a single-window rmac app. Inits gpui-component, opens a chromed window,
/// builds the content view, and wraps it in `Root`.
///
/// ```ignore
/// fn main() {
///     rmac_ui::boot("Text Editor", 900.0, 640.0, |window, cx| EditorView::new(window, cx));
/// }
/// ```
pub fn boot<V, F>(title: impl Into<SharedString>, width: f32, height: f32, build: F)
where
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
{
    let title: SharedString = title.into();
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);

        cx.open_window(window_options(width, height), move |window, cx| {
            let view = cx.new(|cx| build(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
    let _ = title; // reserved for window title once GPUI exposes it post-open
}

/// A full-bleed page background using the active theme — the base every app
/// content sits on, below the title bar.
pub fn page() -> gpui::Div {
    div().size_full().v_flex()
}

/// Convenience: themed background color for the app body.
pub fn body_bg(cx: &App) -> gpui::Hsla {
    cx.theme().background
}
