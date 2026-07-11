//! `rmac-ui` — the shared design system for the rmac desktop suite.
//!
//! Every rmac app depends on this crate so they share one look: macOS-style
//! window chrome (traffic lights), a common theme, fonts, and a `boot` helper
//! that removes the per-app GPUI/Window/Root boilerplate.
//!
//! Apps render `rmac_ui::title_bar(...)` at the top of their view and call
//! `rmac_ui::boot(...)` from `main()`.

use std::{env, fs};

use gpui::{
    div, point, px, rgb, rgba, size, App, AppContext as _, Application, Bounds, Context, ElementId,
    Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use gpui_component::{Root, TitleBar};

mod components;
pub mod theme;
pub use components::{
    alert, dialog, dialog_button, ContextMenu, DialogButtonKind, DismissMenu, RequestClose,
};

// Re-exports so apps depend on one crate for theming; these also bring the
// traits into scope here for `.v_flex()`, `cx.theme()`, etc.
pub use gpui_component::{ActiveTheme, StyledExt};

/// Preferred UI font. Substitute for SF Pro — never ship Apple fonts.
/// Falls back to the platform default if not installed (font embedding lands later).
pub const UI_FONT: &str = "Inter";
/// Preferred monospace font (Terminal, Text Editor, code).
pub const MONO_FONT: &str = "JetBrains Mono";

const BENCHMARK_READY_FILE_ENV: &str = "RMAC_BENCHMARK_READY_FILE";

/// Writes an opt-in marker after the window completes its first frame.
///
/// The performance harness sets the environment variable. Normal application
/// launches do not set it and perform no filesystem I/O.
fn mark_benchmark_first_frame(window: &Window) {
    let Some(path) = env::var_os(BENCHMARK_READY_FILE_ENV) else {
        return;
    };

    window.on_next_frame(move |_, _| {
        fs::write(&path, b"ready\n")
            .unwrap_or_else(|error| panic!("write benchmark marker {path:?}: {error}"));
    });
}

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
            // Push the OS traffic lights off-screen — rmac draws its own in the
            // title bar (see `title_bar`). The window keeps a full-size content
            // view so our chrome draws to the top edge.
            traffic_light_position: Some(point(px(-200.0), px(0.0))),
        }),
        ..Default::default()
    }
}

/// Window options for an app with a **unified 52pt toolbar** (Finder-style):
/// our own traffic lights are drawn by the app's toolbar.
pub fn window_options_unified(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(200.0), px(120.0)),
            size(px(width), px(height)),
        ))),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            // OS traffic lights hidden off-screen; rmac draws its own.
            traffic_light_position: Some(point(px(-200.0), px(0.0))),
        }),
        ..Default::default()
    }
}

/// Like [`boot_with_assets`] but for unified-toolbar apps (Finder, System
/// Settings) — the window reserves a taller titlebar and positions the traffic
/// lights for a 52pt bar. The app renders its own toolbar at the top.
pub fn boot_unified_with_assets<A, V, F>(assets: A, width: f32, height: f32, build: F)
where
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
                let root = cx.new(|cx| Root::new(view, window, cx));
                mark_benchmark_first_frame(window);
                root
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}

/// One traffic-light button: a colored circle that reveals its glyph on hover
/// and runs `on_click` (a window-control action). The glyph is always present
/// but transparent until hover, giving the macOS reveal-on-hover effect.
fn traffic_light(
    id: impl Into<ElementId>,
    color: Hsla,
    glyph: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(12.0))
        .rounded_full()
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.0))
        .font_weight(mac::BOLD)
        .text_color(rgba(0x00000000))
        .hover(|s| s.text_color(rgba(0x00000088)))
        .child(glyph)
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// The rmac traffic-light cluster (close / minimize / zoom), wired to the GPUI
/// window controls. Reusable so unified-toolbar apps can place it themselves.
pub fn traffic_lights() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(traffic_light(
            "tl-close",
            rgb(0xff5f57).into(),
            "✕",
            // Route through the app's close guard (e.g. unsaved-changes prompt)
            // rather than closing the window directly. Apps bind `RequestClose`.
            |window, cx| window.dispatch_action(Box::new(components::RequestClose), cx),
        ))
        .child(traffic_light(
            "tl-min",
            rgb(0xfebc2e).into(),
            "—",
            |window, _| window.minimize_window(),
        ))
        .child(traffic_light(
            "tl-zoom",
            rgb(0x28c840).into(),
            "+",
            |window, _| window.zoom_window(),
        ))
}

/// Overlay our traffic lights in the left gutter (x=13) of a `TitleBar`. The
/// `TitleBar` forces its own children into an 80px left-padded zone, so the
/// lights are layered as a sibling anchored to the bar's true left edge.
fn with_traffic_lights(bar: impl IntoElement) -> impl IntoElement {
    div().relative().w_full().flex_shrink_0().child(bar).child(
        div()
            .absolute()
            .left(px(13.0))
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .child(traffic_lights()),
    )
}

/// The shared title bar: our own traffic lights on the left, centered title.
/// Apps put this at the top of their root `div`. The bar stays draggable via
/// gpui-component's `TitleBar` container.
pub fn title_bar(title: impl Into<SharedString>) -> impl IntoElement {
    let title: SharedString = title.into();
    with_traffic_lights(
        TitleBar::new().child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .child(title),
        ),
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
                let root = cx.new(|cx| Root::new(view, window, cx));
                mark_benchmark_first_frame(window);
                root
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

/// Compatibility accessors for the original light palette. New and migrated
/// views consume [`theme::ThemeTokens`] from their live appearance snapshot;
/// these functions keep existing apps source-compatible during that rollout.
pub mod mac {
    use gpui::{FontWeight, Hsla};

    use crate::theme::ThemeTokens;

    // Surfaces
    /// Window / editor content background.
    pub fn window() -> Hsla {
        ThemeTokens::light_default().colors.window.hsla()
    }
    /// Unified toolbar / window chrome.
    pub fn chrome() -> Hsla {
        ThemeTokens::light_default().colors.chrome.hsla()
    }
    /// Source list (sidebar) background.
    pub fn sidebar() -> Hsla {
        ThemeTokens::light_default().colors.sidebar.hsla()
    }
    /// Middle list column background.
    pub fn list() -> Hsla {
        ThemeTokens::light_default().colors.list.hsla()
    }

    // Text
    /// Primary label color (near-black).
    pub fn text() -> Hsla {
        ThemeTokens::light_default().colors.text.hsla()
    }
    /// Secondary label (systemGray).
    pub fn text_secondary() -> Hsla {
        ThemeTokens::light_default().colors.text_secondary.hsla()
    }
    /// Tertiary label (section headers, counts).
    pub fn text_tertiary() -> Hsla {
        ThemeTokens::light_default().colors.text_tertiary.hsla()
    }

    // Lines & fills
    /// Hairline separator (~8% black).
    pub fn separator() -> Hsla {
        ThemeTokens::light_default().colors.separator.hsla()
    }
    /// Hover fill on rows/controls.
    pub fn hover() -> Hsla {
        ThemeTokens::light_default().colors.hover.hsla()
    }
    /// Neutral (unfocused) selection fill in source lists.
    pub fn sidebar_selection() -> Hsla {
        ThemeTokens::light_default()
            .colors
            .selection_unfocused
            .hsla()
    }

    // Accents (shared by buttons, menus, selections)
    /// System blue — primary actions, selection, focus.
    pub fn accent() -> Hsla {
        ThemeTokens::light_default().colors.accent.hsla()
    }
    /// System red — destructive actions.
    pub fn danger() -> Hsla {
        ThemeTokens::light_default().colors.danger.hsla()
    }
    /// On-accent text (white).
    pub fn on_accent() -> Hsla {
        ThemeTokens::light_default().colors.on_accent.hsla()
    }
    /// Scrim behind a modal dialog (~22% black).
    pub fn scrim() -> Hsla {
        ThemeTokens::light_default().colors.scrim.hsla()
    }

    // Notes accent family (yellow)
    pub fn notes_accent() -> Hsla {
        ThemeTokens::light_default().colors.notes_accent.hsla()
    }
    /// Soft yellow row highlight for the selected note (focused).
    pub fn notes_selection() -> Hsla {
        ThemeTokens::light_default().colors.notes_selection.hsla()
    }

    // Type weights (SF on macOS via the system font)
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
    pub const SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
    pub const BOLD: FontWeight = FontWeight::BOLD;
}

/// A unified macOS toolbar/title bar with the chrome color and a hairline base.
/// Children are laid out after the 80px traffic-light gutter, into which our own
/// traffic lights are drawn (absolutely anchored at the window's top-left).
pub fn toolbar(children: impl IntoElement) -> impl IntoElement {
    use gpui::Styled as _;
    with_traffic_lights(
        TitleBar::new()
            .bg(mac::chrome())
            .border_color(mac::separator())
            .child(children),
    )
}
