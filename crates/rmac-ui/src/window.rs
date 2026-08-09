use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    point, px, size, App, AppContext as _, Application, Bounds, Context, Pixels, Render,
    SharedString, Size, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_component::Root;
use rmac_window_state::{DisplayBounds, Store as WindowStateStore, WindowMode, WindowState};

use crate::{init_application, install_app_menu, prepare_surface_window};

const MIN_WINDOW_WIDTH: f32 = 640.0;
const MIN_WINDOW_HEIGHT: f32 = 360.0;
const WINDOW_STATE_QUIET_PERIOD: Duration = Duration::from_millis(250);
const MAX_NATIVE_TITLE_BYTES: usize = 256;

/// Compose a bounded, single-line compositor title without allowing document,
/// folder, or terminal text to spoof surrounding desktop chrome.
pub fn native_window_title(subject: &str, application: &str) -> String {
    let application = normalize_title_fragment(application, MAX_NATIVE_TITLE_BYTES);
    if application.is_empty() {
        return String::new();
    }
    let separator = " — ";
    let subject_limit = MAX_NATIVE_TITLE_BYTES
        .saturating_sub(separator.len())
        .saturating_sub(application.len());
    let subject = normalize_title_fragment(subject, subject_limit);
    if subject.is_empty() || subject == application {
        application
    } else {
        format!("{subject}{separator}{application}")
    }
}

fn normalize_title_fragment(value: &str, limit: usize) -> String {
    let mut output = String::with_capacity(value.len().min(limit));
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
        {
            continue;
        }
        if pending_space {
            if output.len() + 1 > limit {
                break;
            }
            output.push(' ');
            pending_space = false;
        }
        if output.len() + character.len_utf8() > limit {
            break;
        }
        output.push(character);
    }
    output
}

fn minimum_window_size(width: f32, height: f32) -> Size<Pixels> {
    size(
        px(width.min(MIN_WINDOW_WIDTH)),
        px(height.min(MIN_WINDOW_HEIGHT)),
    )
}

fn centered_window_bounds(width: f32, height: f32, cx: &App) -> WindowBounds {
    WindowBounds::centered(size(px(width), px(height)), cx)
}

fn restored_window_bounds(app_id: &str, width: f32, height: f32, cx: &App) -> WindowBounds {
    let fallback = || centered_window_bounds(width, height, cx);
    let Some(state) = WindowStateStore::from_environment(app_id)
        .and_then(|store| store.load())
        .ok()
        .flatten()
    else {
        return fallback();
    };
    let displays = connected_display_bounds(cx);
    let minimum = minimum_window_size(width, height);
    let Some(state) = state.fit_to_displays(
        &displays,
        f64::from(minimum.width),
        f64::from(minimum.height),
    ) else {
        return fallback();
    };
    let bounds = Bounds::new(
        point(px(state.x as f32), px(state.y as f32)),
        size(px(state.width as f32), px(state.height as f32)),
    );
    match state.mode {
        WindowMode::Windowed => WindowBounds::Windowed(bounds),
        WindowMode::Maximized => WindowBounds::Maximized(bounds),
        WindowMode::Fullscreen => WindowBounds::Fullscreen(bounds),
    }
}

fn connected_display_bounds(cx: &App) -> Vec<DisplayBounds> {
    cx.primary_display()
        .into_iter()
        .chain(cx.displays())
        .filter_map(|display| {
            let bounds = display.bounds();
            DisplayBounds::checked(
                f64::from(bounds.origin.x),
                f64::from(bounds.origin.y),
                f64::from(bounds.size.width),
                f64::from(bounds.size.height),
            )
        })
        .collect()
}

fn window_options_with_bounds(
    width: f32,
    height: f32,
    window_bounds: WindowBounds,
    title: Option<SharedString>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(window_bounds),
        titlebar: Some(TitlebarOptions {
            title,
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
    window_options_with_bounds(
        width,
        height,
        centered_window_bounds(width, height, cx),
        None,
    )
}

/// Standard window options with a stable Linux desktop identity.
pub fn window_options_for_app(app_id: &str, width: f32, height: f32, cx: &App) -> WindowOptions {
    WindowOptions {
        app_id: Some(app_id.to_owned()),
        ..window_options_with_bounds(
            width,
            height,
            restored_window_bounds(app_id, width, height, cx),
            rmac_apps::identity::window_title(app_id).map(SharedString::from),
        )
    }
}

/// Identified window options with an explicit native title. This is useful for
/// document windows whose compositor title is more specific than the stable
/// application name.
pub fn window_options_for_app_with_title(
    app_id: &str,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    cx: &App,
) -> WindowOptions {
    WindowOptions {
        app_id: Some(app_id.to_owned()),
        ..window_options_with_bounds(
            width,
            height,
            restored_window_bounds(app_id, width, height, cx),
            Some(title.into()),
        )
    }
}

/// Window options for an app with a **unified 52pt toolbar** (Finder-style):
/// our own traffic lights are drawn by the app's toolbar.
pub fn window_options_unified(width: f32, height: f32, cx: &App) -> WindowOptions {
    window_options_with_bounds(
        width,
        height,
        centered_window_bounds(width, height, cx),
        None,
    )
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
        ..window_options_with_bounds(
            width,
            height,
            restored_window_bounds(app_id, width, height, cx),
            rmac_apps::identity::window_title(app_id).map(SharedString::from),
        )
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
        ..window_options_with_bounds(
            width,
            height,
            window_bounds,
            rmac_apps::identity::window_title(app_id).map(SharedString::from),
        )
    }
}

/// Observe one app window and durably save only its latest stable geometry.
/// Resize bursts coalesce for a short quiet period, and persistence runs away
/// from the render thread. Failure is deliberately non-fatal: geometry is a
/// convenience and the next launch falls back to safe centered bounds.
pub fn observe_window_state<V: 'static>(app_id: &str, window: &mut Window, cx: &Context<V>) {
    let Ok(store) = WindowStateStore::from_environment(app_id) else {
        return;
    };
    let pending = Arc::new(Mutex::new(None));
    let (wake, events) = async_channel::bounded(1);
    queue_window_state(window, &pending, &wake);

    let observer_pending = Arc::clone(&pending);
    let observer_wake = wake.clone();
    cx.observe_window_bounds(window, move |_, window, _| {
        queue_window_state(window, &observer_pending, &observer_wake);
    })
    .detach();
    drop(wake);

    cx.background_executor()
        .spawn(async move {
            while events.recv().await.is_ok() {
                async_io::Timer::after(WINDOW_STATE_QUIET_PERIOD).await;
                while events.try_recv().is_ok() {}
                let state = pending.lock().ok().and_then(|mut pending| pending.take());
                if let Some(state) = state {
                    let _ = store.save(state);
                }
            }
        })
        .detach();
}

fn queue_window_state(
    window: &Window,
    pending: &Arc<Mutex<Option<WindowState>>>,
    wake: &async_channel::Sender<()>,
) {
    let bounds = window.window_bounds();
    let mode = match bounds {
        WindowBounds::Windowed(_) => WindowMode::Windowed,
        WindowBounds::Maximized(_) => WindowMode::Maximized,
        WindowBounds::Fullscreen(_) => WindowMode::Fullscreen,
    };
    let bounds = bounds.get_bounds();
    let Some(state) = WindowState::checked(
        f64::from(bounds.origin.x),
        f64::from(bounds.origin.y),
        f64::from(bounds.size.width),
        f64::from(bounds.size.height),
        mode,
    ) else {
        return;
    };
    if let Ok(mut pending) = pending.lock() {
        *pending = Some(state);
        let _ = wake.try_send(());
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
            install_app_menu(app_id, cx);
            let options = window_options_unified_for_app(app_id, width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    observe_window_state(app_id, window, cx);
                    build(window, cx)
                });
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
    let fallback_title: SharedString = title.into();
    let title = rmac_apps::identity::window_title(app_id)
        .map(SharedString::from)
        .unwrap_or(fallback_title);
    Application::new()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            install_app_menu(app_id, cx);
            let options = window_options_for_app_with_title(app_id, title, width, height, cx);

            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    observe_window_state(app_id, window, cx);
                    build(window, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
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
            let options = window_options_with_bounds(
                width,
                height,
                centered_window_bounds(width, height, cx),
                Some(title),
            );

            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}
