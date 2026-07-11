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
mod controls;
pub mod gallery;
pub mod theme;
pub use components::{
    alert, dialog, dialog_button, ContextMenu, DialogButtonKind, DismissMenu, RequestClose,
};
pub use controls::{
    Button, ButtonRole, CollectionState, InputState, List, ListRow, SearchField, Slider,
    SliderAxis, SliderEvent, SliderState, Table, Tabs, TextField, Toggle, ToggleState, Tree,
    TreeRow,
};
pub use controls::{Column, ColumnSort, TableDelegate, TableEvent, TableState};

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
            start_theme_runtime(cx);
            cx.open_window(window_options_unified(width, height), move |window, cx| {
                gpui_component::theme::Theme::change(
                    current_component_theme_mode(),
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
            start_theme_runtime(cx);

            cx.open_window(window_options(width, height), move |window, cx| {
                gpui_component::theme::Theme::change(
                    current_component_theme_mode(),
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

fn start_theme_runtime(cx: &mut App) {
    // Initial resolution stays off the first-frame path.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async { load_resolved_tokens().await })
            .await;
        if let Ok(tokens) = result {
            apply_resolved_tokens(tokens, cx);
        }
    })
    .detach();

    // Host appearance changes are already reconnecting and report complete
    // snapshots, so consumers never need to merge partial portal state.
    let (portal_tx, portal_rx) = async_channel::bounded(8);
    cx.background_executor()
        .spawn(async move { rmac_appearance_portal::watch(portal_tx).await })
        .detach();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(event) = portal_rx.recv().await {
            let rmac_appearance::Event::Snapshot(host) = event else {
                continue;
            };
            let result = cx
                .background_executor()
                .spawn(async move { load_tokens_with_host(host) })
                .await;
            if let Ok(tokens) = result {
                apply_resolved_tokens(tokens, cx);
            }
        }
    })
    .detach();

    // Preference writes from System Settings arrive through the bounded file
    // watcher. Re-read the host as well so automatic values cannot go stale.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let watcher = cx
            .background_executor()
            .spawn(async {
                rmac_theme::ThemeStore::from_environment()
                    .and_then(|store| store.watch())
                    .map_err(|error| error.to_string())
            })
            .await;
        let Ok(watcher) = watcher else {
            return;
        };
        while let Ok(event) = watcher.recv().await {
            if !matches!(event, rmac_theme::StoreEvent::Changed) {
                continue;
            }
            let result = cx
                .background_executor()
                .spawn(async { load_resolved_tokens().await })
                .await;
            if let Ok(tokens) = result {
                apply_resolved_tokens(tokens, cx);
            }
        }
    })
    .detach();
}

async fn load_resolved_tokens() -> Result<theme::ThemeTokens, String> {
    let host = match rmac_appearance_portal::snapshot().await {
        Ok(host) => host,
        Err(error) => rmac_appearance::Snapshot::unavailable(error.to_string()),
    };
    load_tokens_with_host(host)
}

fn load_tokens_with_host(host: rmac_appearance::Snapshot) -> Result<theme::ThemeTokens, String> {
    let store = rmac_theme::ThemeStore::from_environment().map_err(|error| error.to_string())?;
    let resolved = store.load(&host).map_err(|error| error.to_string())?;
    Ok(theme::ThemeTokens::from_appearance(resolved.effective))
}

fn apply_resolved_tokens(tokens: theme::ThemeTokens, cx: &mut gpui::AsyncApp) {
    if !theme::set_current(tokens) {
        return;
    }
    let mode = component_theme_mode(tokens.color_scheme);
    let _ = cx.update(|app| {
        gpui_component::theme::Theme::change(mode, None, app);
        app.refresh_windows();
    });
}

fn current_component_theme_mode() -> gpui_component::theme::ThemeMode {
    component_theme_mode(theme::current().color_scheme)
}

fn component_theme_mode(
    scheme: rmac_appearance::ResolvedColorScheme,
) -> gpui_component::theme::ThemeMode {
    match scheme {
        rmac_appearance::ResolvedColorScheme::Light => gpui_component::theme::ThemeMode::Light,
        rmac_appearance::ResolvedColorScheme::Dark => gpui_component::theme::ThemeMode::Dark,
    }
}

/// Compatibility accessors for the original light palette. New and migrated
/// views consume [`theme::ThemeTokens`] from their live appearance snapshot;
/// these functions keep existing apps source-compatible during that rollout.
pub mod mac {
    use gpui::{FontWeight, Hsla};

    // Surfaces
    /// Window / editor content background.
    pub fn window() -> Hsla {
        crate::theme::current().colors.window.hsla()
    }
    /// Raised card, popover, and compact overlay background.
    pub fn raised() -> Hsla {
        crate::theme::current().colors.raised.hsla()
    }
    /// Unified toolbar / window chrome.
    pub fn chrome() -> Hsla {
        crate::theme::current().colors.chrome.hsla()
    }
    /// Source list (sidebar) background.
    pub fn sidebar() -> Hsla {
        crate::theme::current().colors.sidebar.hsla()
    }
    /// Middle list column background.
    pub fn list() -> Hsla {
        crate::theme::current().colors.list.hsla()
    }

    // Text
    /// Primary label color (near-black).
    pub fn text() -> Hsla {
        crate::theme::current().colors.text.hsla()
    }
    /// Secondary label (systemGray).
    pub fn text_secondary() -> Hsla {
        crate::theme::current().colors.text_secondary.hsla()
    }
    /// Tertiary label (section headers, counts).
    pub fn text_tertiary() -> Hsla {
        crate::theme::current().colors.text_tertiary.hsla()
    }

    // Lines & fills
    /// Hairline separator (~8% black).
    pub fn separator() -> Hsla {
        crate::theme::current().colors.separator.hsla()
    }
    /// Hover fill on rows/controls.
    pub fn hover() -> Hsla {
        crate::theme::current().colors.hover.hsla()
    }
    pub fn row_alternate() -> Hsla {
        crate::theme::current().colors.row_alternate.hsla()
    }
    pub fn control_fill() -> Hsla {
        crate::theme::current().colors.control_fill.hsla()
    }
    pub fn control_fill_hover() -> Hsla {
        crate::theme::current().colors.control_fill_hover.hsla()
    }
    /// Neutral (unfocused) selection fill in source lists.
    pub fn sidebar_selection() -> Hsla {
        crate::theme::current().colors.selection_unfocused.hsla()
    }

    // Accents (shared by buttons, menus, selections)
    /// System blue — primary actions, selection, focus.
    pub fn accent() -> Hsla {
        crate::theme::current().colors.accent.hsla()
    }
    pub fn accent_subtle() -> Hsla {
        crate::theme::current().colors.accent_subtle.hsla()
    }
    pub fn accent_border() -> Hsla {
        crate::theme::current().colors.accent_border.hsla()
    }
    /// System red — destructive actions.
    pub fn danger() -> Hsla {
        crate::theme::current().colors.danger.hsla()
    }
    /// Legible text over the destructive fill.
    pub fn on_danger() -> Hsla {
        crate::theme::current().colors.on_danger.hsla()
    }
    pub fn error_background() -> Hsla {
        crate::theme::current().colors.error_background.hsla()
    }
    pub fn error_border() -> Hsla {
        crate::theme::current().colors.error_border.hsla()
    }
    pub fn warning_background() -> Hsla {
        crate::theme::current().colors.warning_background.hsla()
    }
    pub fn warning_border() -> Hsla {
        crate::theme::current().colors.warning_border.hsla()
    }
    pub fn warning_text() -> Hsla {
        crate::theme::current().colors.warning_text.hsla()
    }
    /// On-accent text (white).
    pub fn on_accent() -> Hsla {
        crate::theme::current().colors.on_accent.hsla()
    }
    /// Scrim behind a modal dialog (~22% black).
    pub fn scrim() -> Hsla {
        crate::theme::current().colors.scrim.hsla()
    }

    // Notes accent family (yellow)
    pub fn notes_accent() -> Hsla {
        crate::theme::current().colors.notes_accent.hsla()
    }
    /// Soft yellow row highlight for the selected note (focused).
    pub fn notes_selection() -> Hsla {
        crate::theme::current().colors.notes_selection.hsla()
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
