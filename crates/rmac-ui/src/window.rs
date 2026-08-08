use gpui::{
    point, px, size, App, AppContext as _, Application, Context, Pixels, Render, SharedString,
    Size, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_component::Root;

use crate::{init_application, prepare_surface_window};

const MIN_WINDOW_WIDTH: f32 = 640.0;
const MIN_WINDOW_HEIGHT: f32 = 360.0;

fn minimum_window_size(width: f32, height: f32) -> Size<Pixels> {
    size(
        px(width.min(MIN_WINDOW_WIDTH)),
        px(height.min(MIN_WINDOW_HEIGHT)),
    )
}

fn centered_window_bounds(width: f32, height: f32, cx: &App) -> WindowBounds {
    WindowBounds::centered(size(px(width), px(height)), cx)
}

fn window_options_with_bounds(
    width: f32,
    height: f32,
    window_bounds: WindowBounds,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(window_bounds),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            // Push the OS traffic lights off-screen — rmac draws its own in the
            // title bar (see `title_bar`). The window keeps a full-size content
            // view so our chrome draws to the top edge.
            traffic_light_position: Some(point(px(-200.0), px(0.0))),
        }),
        window_min_size: Some(minimum_window_size(width, height)),
        ..Default::default()
    }
}

/// Standard window options for an rmac app window: macOS traffic lights in the
/// canonical position, transparent titlebar so our chrome draws through, and
/// first-launch placement centered on the active primary display.
pub fn window_options(width: f32, height: f32, cx: &App) -> WindowOptions {
    window_options_with_bounds(width, height, centered_window_bounds(width, height, cx))
}

/// Standard window options with a stable Linux desktop identity.
pub fn window_options_for_app(app_id: &str, width: f32, height: f32, cx: &App) -> WindowOptions {
    WindowOptions {
        app_id: Some(app_id.to_owned()),
        ..window_options(width, height, cx)
    }
}

/// Window options for an app with a **unified 52pt toolbar** (Finder-style):
/// our own traffic lights are drawn by the app's toolbar.
pub fn window_options_unified(width: f32, height: f32, cx: &App) -> WindowOptions {
    window_options_with_bounds(width, height, centered_window_bounds(width, height, cx))
}

/// Unified-toolbar options with a stable Linux desktop identity.
pub fn window_options_unified_for_app(
    app_id: &str,
    width: f32,
    height: f32,
    cx: &App,
) -> WindowOptions {
    WindowOptions {
        app_id: Some(app_id.to_owned()),
        ..window_options_unified(width, height, cx)
    }
}

#[cfg(test)]
pub(crate) fn window_options_for_app_with_bounds(
    app_id: &str,
    width: f32,
    height: f32,
    window_bounds: WindowBounds,
) -> WindowOptions {
    WindowOptions {
        app_id: Some(app_id.to_owned()),
        ..window_options_with_bounds(width, height, window_bounds)
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
            init_application(cx);
            let options = window_options_unified(width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}

/// [`boot_unified_with_assets`] with an explicit desktop/Wayland identity.
pub fn boot_unified_app_with_assets<A, V, F>(
    app_id: &'static str,
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
            init_application(cx);
            let options = window_options_unified_for_app(app_id, width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
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

/// [`boot`] with an explicit desktop/Wayland identity.
pub fn boot_app<V, F>(
    app_id: &'static str,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    build: F,
) where
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
{
    boot_app_with_assets(
        app_id,
        gpui_component_assets::Assets,
        title,
        width,
        height,
        build,
    );
}

/// [`boot_with_assets`] with an explicit desktop/Wayland identity.
pub fn boot_app_with_assets<A, V, F>(
    app_id: &'static str,
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
            init_application(cx);
            let options = window_options_for_app(app_id, width, height, cx);

            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
    let _ = title;
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
            init_application(cx);
            let options = window_options(width, height, cx);

            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
    let _ = title; // reserved for window title once GPUI exposes it post-open
}
