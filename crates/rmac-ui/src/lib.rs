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

/// Window options for an app with a **unified 52pt toolbar** (Finder-style):
/// traffic lights positioned for the taller bar.
pub fn window_options_unified(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(200.0), px(120.0)),
            size(px(width), px(height)),
        ))),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(19.0), px(19.0))),
        }),
        ..Default::default()
    }
}

/// Like [`boot_with_assets`] but for unified-toolbar apps (Finder, System
/// Settings) — the window reserves a taller titlebar and positions the traffic
/// lights for a 52pt bar. The app renders its own toolbar at the top.
pub fn boot_unified_with_assets<A, V, F>(
    assets: A,
    width: f32,
    height: f32,
    build: F,
) where
    A: gpui::AssetSource,
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
{
    Application::new()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            cx.open_window(window_options_unified(width, height), move |window, cx| {
                gpui_component::theme::Theme::change(
                    gpui_component::theme::ThemeMode::Light,
                    Some(window),
                    cx,
                );
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
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
    boot_with_assets(gpui_component_assets::Assets, title, width, height, build);
}

/// Like [`boot`], but with a custom asset source (e.g. an app that embeds its
/// own SVG icons combined with gpui-component's). The source must still resolve
/// gpui-component's `icons/**` paths or built-in icons won't render.
pub fn boot_with_assets<A, V, F>(
    assets: A,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    build: F,
) where
    A: gpui::AssetSource,
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
{
    let title: SharedString = title.into();
    Application::new()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);

            cx.open_window(window_options(width, height), move |window, cx| {
                // Force light mode so every built-in widget matches the `mac` palette.
                gpui_component::theme::Theme::change(
                    gpui_component::theme::ThemeMode::Light,
                    Some(window),
                    cx,
                );
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

/// Precise macOS (light-mode) design tokens — system colors, weights, metrics.
/// Apps use these instead of generic theme colors so the suite matches macOS
/// pixel-for-pixel. (Dark mode + dynamic switching is a later pass.)
pub mod mac {
    use gpui::{rgb, rgba, FontWeight, Hsla};

    // Surfaces
    /// Window / editor content background.
    pub fn window() -> Hsla { rgb(0xffffff).into() }
    /// Unified toolbar / window chrome.
    pub fn chrome() -> Hsla { rgb(0xf6f6f6).into() }
    /// Source list (sidebar) background.
    pub fn sidebar() -> Hsla { rgb(0xf2f2f2).into() }
    /// Middle list column background.
    pub fn list() -> Hsla { rgb(0xffffff).into() }

    // Text
    /// Primary label color (near-black).
    pub fn text() -> Hsla { rgb(0x1d1d1f).into() }
    /// Secondary label (systemGray).
    pub fn text_secondary() -> Hsla { rgb(0x86868b).into() }
    /// Tertiary label (section headers, counts).
    pub fn text_tertiary() -> Hsla { rgb(0xaeaeb2).into() }

    // Lines & fills
    /// Hairline separator (~8% black).
    pub fn separator() -> Hsla { rgba(0x00000014).into() }
    /// Hover fill on rows/controls.
    pub fn hover() -> Hsla { rgba(0x0000000a).into() }
    /// Neutral (unfocused) selection fill in source lists.
    pub fn sidebar_selection() -> Hsla { rgba(0x00000014).into() }

    // Notes accent family (yellow)
    pub fn notes_accent() -> Hsla { rgb(0xffc40c).into() }
    /// Soft yellow row highlight for the selected note (focused).
    pub fn notes_selection() -> Hsla { rgb(0xfdeaa3).into() }

    // Type weights (SF on macOS via the system font)
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
    pub const SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
    pub const BOLD: FontWeight = FontWeight::BOLD;
}

/// A unified macOS toolbar/title bar with the chrome color and a hairline base.
/// Children are laid out after the 80px traffic-light gutter.
pub fn toolbar(children: impl IntoElement) -> impl IntoElement {
    use gpui::Styled as _;
    TitleBar::new()
        .bg(mac::chrome())
        .border_color(mac::separator())
        .child(children)
}
