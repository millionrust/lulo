use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    point, px, size, App, AppContext as _, Bounds, Context, Decorations, Edges, Pixels, Render,
    SharedString, Size, TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions,
};
// Re-exported (not just imported) so a window opened outside a `boot*` entry
// point — an app's own auxiliary window — can wrap its view without another
// file needing its own `gpui_component` import (ADR 0015 keeps that boundary
// at exactly the files that already cross it).
pub use gpui_component::Root;
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

/// The transparent margin the root view reserves around an app window on
/// Linux for its client-side shadow and resize edges (gpui-component's Root
/// default). GPUI's window bounds include it; the compositor's window
/// geometry — the window the user sees — does not.
#[cfg(target_os = "linux")]
const CLIENT_FRAME_INSET: f32 = 12.0;
#[cfg(not(target_os = "linux"))]
const CLIENT_FRAME_INSET: f32 = 0.0;

/// Outer window bounds for a visible window of `width` × `height`.
pub fn outer_window_size(width: f32, height: f32) -> (f32, f32) {
    (
        width + 2.0 * CLIENT_FRAME_INSET,
        height + 2.0 * CLIENT_FRAME_INSET,
    )
}

/// Tell the platform about the client frame before the first configure, so
/// the compositor's first window geometry already excludes it and a new
/// window opens at exactly the size the app asked for. Every window built
/// with a `Root` (see the re-export above) needs this: Root reserves the
/// same margin for its client-side shadow and resize edges regardless of
/// whether the platform was told about it, so skipping this call leaves
/// Root's content box smaller than the window the caller asked for and the
/// edges of that content — often exactly the bottom row of buttons — clip
/// against the visible window instead of sitting inside it.
pub fn reserve_client_frame(window: &mut Window) {
    if CLIENT_FRAME_INSET > 0.0 {
        window.set_client_inset(px(CLIENT_FRAME_INSET));
    }
}

/// How far the visible window sits inside GPUI's window bounds on each side:
/// the client frame on untiled edges, nothing under server decorations.
pub fn window_content_insets(window: &Window) -> Edges<Pixels> {
    match window.window_decorations() {
        Decorations::Server => Edges::all(px(0.0)),
        Decorations::Client { tiling } => {
            let inset = window.client_inset().unwrap_or(px(0.0));
            let edge = |tiled: bool| if tiled { px(0.0) } else { inset };
            Edges {
                top: edge(tiling.top),
                right: edge(tiling.right),
                bottom: edge(tiling.bottom),
                left: edge(tiling.left),
            }
        }
    }
}

/// The size of the window the user sees — `viewport_size` less the client
/// frame. Layout that fills the window (a terminal grid) must use this.
pub fn window_content_size(window: &Window) -> Size<Pixels> {
    let viewport = window.viewport_size();
    let insets = window_content_insets(window);
    size(
        (viewport.width - insets.left - insets.right).max(px(0.0)),
        (viewport.height - insets.top - insets.bottom).max(px(0.0)),
    )
}

fn minimum_window_size(width: f32, height: f32) -> Size<Pixels> {
    size(
        px(width.min(MIN_WINDOW_WIDTH)),
        px(height.min(MIN_WINDOW_HEIGHT)),
    )
}

fn centered_window_bounds(width: f32, height: f32, cx: &App) -> WindowBounds {
    let (width, height) = cx
        .primary_display()
        .map(|display| {
            let screen = display.bounds().size;
            fit_to_screen(
                width,
                height,
                f32::from(screen.width),
                f32::from(screen.height),
            )
        })
        .unwrap_or((width, height));
    WindowBounds::centered(size(px(width), px(height)), cx)
}

/// GPUI cannot report the logical screen size on Wayland before or right
/// after mapping (it names no display, lists outputs divided by wl_output's
/// integer scale, and briefly reports that integer scale), so ask the
/// compositor for the focused output's logical size and shrink the new window
/// if it reaches under the Dock.
fn fit_to_display_after_first_frame(window: &Window, cx: &App) {
    window
        .spawn(cx, async move |cx| {
            let Ok(snapshot) = rmac_compositor_niri::snapshot().await else {
                return;
            };
            let screen = snapshot
                .focus
                .output
                .as_ref()
                .and_then(|id| snapshot.outputs.iter().find(|output| &output.id == id))
                .or_else(|| snapshot.outputs.first())
                .and_then(|output| output.logical.as_ref())
                .map(|logical| (logical.size.width as f32, logical.size.height as f32));
            let Some((screen_width, screen_height)) = screen else {
                return;
            };
            // This process's own newly mapped window (WIN-02): niri centres
            // a *resize* on the working area below the menu bar and beside
            // the Dock (the same area `Action::CenterWindow` and the green
            // button's Centre use), but centres a window's *initial*
            // placement on the whole output. Even a window that fits can
            // therefore open under the Dock unless it is centred after map.
            let pid = std::process::id() as i32;
            let window_id = snapshot
                .windows
                .iter()
                .filter(|window| window.pid == Some(pid))
                .min_by_key(|window| i32::from(!window.focused))
                .map(|window| window.id);
            // niri configures a new floating window after it maps and would
            // replace a size set before that, so keep fitting briefly until
            // the window stays inside the space above the Dock.
            for _ in 0..10 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let oversized = cx.update(|window, _| {
                    let current = window.bounds().size;
                    let (width, height) = fit_to_screen(
                        f32::from(current.width),
                        f32::from(current.height),
                        screen_width,
                        screen_height,
                    );
                    let oversized =
                        width < f32::from(current.width) || height < f32::from(current.height);
                    if oversized {
                        window.resize(size(px(width), px(height)));
                    }
                    oversized
                });
                if let Some(window_id) = window_id {
                    let _ = rmac_compositor_niri::execute_action(
                        &rmac_compositor::Action::CenterWindow { window: window_id },
                    )
                    .await;
                }
                if !matches!(oversized, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
}

/// macOS never opens a new window taller or wider than the space between the
/// menu bar and the Dock; on a small screen the default size shrinks to fit.
fn fit_to_screen(width: f32, height: f32, screen_width: f32, screen_height: f32) -> (f32, f32) {
    // Menu bar, Dock shelf with its margin, and a little air around both.
    const RESERVED_HEIGHT: f32 = 29.0 + 89.0 + 16.0;
    const SIDE_MARGIN: f32 = 40.0;
    let max_width = (screen_width - SIDE_MARGIN).max(MIN_WINDOW_WIDTH);
    let max_height = (screen_height - RESERVED_HEIGHT).max(MIN_WINDOW_HEIGHT);
    (width.min(max_width), height.min(max_height))
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
    // A size saved on a larger screen still has to fit between the menu bar
    // and the Dock here; floating windows ignore the Dock's reserved zone.
    let (width, height) = match (state.mode, cx.primary_display()) {
        (WindowMode::Windowed, Some(display)) => {
            let screen = display.bounds().size;
            fit_to_screen(
                state.width as f32,
                state.height as f32,
                f32::from(screen.width),
                f32::from(screen.height),
            )
        }
        _ => (state.width as f32, state.height as f32),
    };
    let bounds = Bounds::new(
        point(px(state.x as f32), px(state.y as f32)),
        size(px(width), px(height)),
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
        // rmac owns the complete titlebar, including its single macOS traffic-
        // light cluster. Requesting server decorations on Wayland lets
        // libdecor add a second Linux minimize/maximize/close cluster.
        window_decorations: Some(WindowDecorations::Client),
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
///
/// The window is also tracked as a key-window candidate for the menu bar
/// ([`crate::track_key_window`]).
pub fn observe_window_state<V: 'static>(app_id: &str, window: &mut Window, cx: &Context<V>) {
    crate::menu_target::track_key_window(window, cx);
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
                    if let Err(error) = store.save(state) {
                        eprintln!("could not save the window state: {error}");
                    }
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
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            let options = window_options_unified(width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                fit_to_display_after_first_frame(window, cx);
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
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            install_app_menu(app_id, cx);
            let options = window_options_unified_for_app(app_id, width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                fit_to_display_after_first_frame(window, cx);
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

/// [`boot_unified_app_with_assets`] for an app that keeps every window in
/// one process, as a macOS app does. `windows` holds one argument list per
/// window this launch asks for (none means one default window). If the app
/// is already running, this launch hands each list to it over D-Bus, the
/// running process opens a window for each, and this function returns
/// without starting GPUI. Otherwise it opens the windows itself and serves
/// later launches' requests with `build` too.
pub fn boot_unified_app_instance_with_assets<A, V, F>(
    app_id: &'static str,
    assets: A,
    width: f32,
    height: f32,
    windows: Vec<Vec<String>>,
    build: F,
) where
    A: gpui::AssetSource,
    V: Render + 'static,
    F: Fn(&[String], &mut Window, &mut Context<V>) -> V + 'static,
{
    let mut windows = windows;
    if windows.is_empty() {
        windows.push(Vec::new());
    }
    if hand_off_to_running_instance(app_id, &windows) {
        return;
    }
    let build = Rc::new(build);
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            let requested = build.clone();
            let opener: OpenWindow = Rc::new(move |arguments, cx| {
                if let Err(error) =
                    open_unified_window(app_id, width, height, arguments, requested.clone(), cx)
                {
                    eprintln!("{app_id} could not open a new window: {error}");
                }
            });
            cx.set_global(AppWindowOpener(opener.clone()));
            crate::runtime::install_app_instance(
                app_id,
                move |arguments, cx| opener(arguments, cx),
                cx,
            );
            let mut windows = windows.into_iter();
            if let Some(first) = windows.next() {
                open_unified_window(app_id, width, height, first, build.clone(), cx)
                    .expect("failed to open window");
            }
            for arguments in windows {
                if let Err(error) =
                    open_unified_window(app_id, width, height, arguments, build.clone(), cx)
                {
                    eprintln!("{app_id} could not open another window: {error}");
                }
            }
            cx.activate(true);
        });
}

/// [`boot_unified_app_with_assets`] for a single-window unified-toolbar app
/// (System Settings): unlike [`boot_unified_app_instance_with_assets`], a
/// second launch never opens another window. It hands `arguments` to the
/// running process instead, which passes them to `on_reopen` — so the app
/// can react to what was asked for (for example, navigate to a requested
/// pane) — and then brings its one window forward, exactly as
/// [`boot_single_window_app_with_assets`] does for a chromed window.
pub fn boot_unified_single_window_app_with_assets<A, V, F, H>(
    app_id: &'static str,
    assets: A,
    width: f32,
    height: f32,
    arguments: Vec<String>,
    build: F,
    on_reopen: H,
) where
    A: gpui::AssetSource,
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    H: Fn(Vec<String>, &mut App) + 'static,
{
    if hand_off_to_running_instance(app_id, &[arguments]) {
        focus_running_app(app_id);
        return;
    }
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            crate::install_app_instance(
                app_id,
                move |arguments, cx| {
                    on_reopen(arguments, cx);
                    if let Some((window, _)) = crate::menu_target::target(cx) {
                        let _ = window.update(cx, |_, window, _| window.activate_window());
                    }
                },
                cx,
            );
            let options = window_options_unified_for_app(app_id, width, height, cx);
            cx.open_window(options, move |window, cx| {
                prepare_surface_window(window, cx);
                fit_to_display_after_first_frame(window, cx);
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

/// Send `action` to the window that owns this app's menu — the same
/// dispatch the desktop menu bar uses — focusing its registered content
/// first if the action would not otherwise reach it. For an `on_reopen`
/// callback (see [`boot_unified_single_window_app_with_assets`]) to react to
/// a second launch's arguments with something the app's own view handles
/// through `on_action`.
pub fn dispatch_to_app_window(action: Box<dyn gpui::Action>, cx: &mut App) {
    crate::menu_target::dispatch_menu_action(action, cx);
}

/// Opens a small fixed-size panel of this app (the About panel), centred on
/// the display rather than at the app's remembered window geometry.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn open_panel_window<V, F>(
    app_id: &str,
    title: String,
    width: f32,
    height: f32,
    cx: &mut App,
    build: F,
) -> gpui::Result<gpui::AnyWindowHandle>
where
    V: Render + 'static,
    F: FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
{
    let (outer_width, outer_height) = outer_window_size(width, height);
    let options = WindowOptions {
        app_id: Some(app_id.to_owned()),
        is_resizable: false,
        is_minimizable: false,
        window_min_size: Some(size(px(width), px(height))),
        ..window_options_with_bounds(
            outer_width,
            outer_height,
            centered_window_bounds(outer_width, outer_height, cx),
            Some(SharedString::from(title)),
        )
    };
    let handle = cx.open_window(options, move |window, cx| {
        reserve_client_frame(window);
        prepare_surface_window(window, cx);
        let view = cx.new(|cx| build(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(handle.into())
}

/// Hand this launch's windows to the app's running process, if it has one,
/// so each app runs as one process with one menu, as on the Mac. Returns
/// true when a running process took them and this launch should exit. Each
/// entry is one window's arguments (at most eight, each a path or option
/// that is valid UTF-8); paths must be absolute, since the running process
/// has its own working directory.
///
/// A process that owns the name but does not answer is reported, and this
/// launch then starts on its own rather than show nothing.
pub fn hand_off_to_running_instance(app_id: &'static str, windows: &[Vec<String>]) -> bool {
    #[cfg(target_os = "linux")]
    {
        let first = windows.first().cloned().unwrap_or_default();
        match async_io::block_on(rmac_app_menu::open_window_in_running_instance(
            app_id, &first,
        )) {
            Ok(true) => {
                for arguments in windows.iter().skip(1) {
                    if let Err(error) = async_io::block_on(
                        rmac_app_menu::open_window_in_running_instance(app_id, arguments),
                    ) {
                        eprintln!("{app_id} could not open another window: {error}");
                    }
                }
                true
            }
            Ok(false) => false,
            Err(error) => {
                eprintln!("{app_id} could not reach its running process: {error}");
                false
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app_id, windows);
        false
    }
}

/// Opens one more window from its command-line arguments.
type OpenWindow = Rc<dyn Fn(Vec<String>, &mut App)>;

/// How a one-process app opens another of its windows.
struct AppWindowOpener(OpenWindow);

impl gpui::Global for AppWindowOpener {}

/// File ▸ New Window for an app booted with
/// [`boot_unified_app_instance_with_assets`]: opens another window in this
/// process for `arguments`, exactly as a second launch would. Returns false
/// when the app was not booted that way.
pub fn open_another_window(arguments: Vec<String>, cx: &mut App) -> bool {
    let Some(opener) = cx
        .try_global::<AppWindowOpener>()
        .map(|opener| opener.0.clone())
    else {
        return false;
    };
    opener(arguments, cx);
    true
}

fn open_unified_window<V, F>(
    app_id: &'static str,
    width: f32,
    height: f32,
    arguments: Vec<String>,
    build: Rc<F>,
    cx: &mut App,
) -> gpui::Result<()>
where
    V: Render + 'static,
    F: Fn(&[String], &mut Window, &mut Context<V>) -> V + 'static,
{
    let options = window_options_unified_for_app(app_id, width, height, cx);
    cx.open_window(options, move |window, cx| {
        prepare_surface_window(window, cx);
        fit_to_display_after_first_frame(window, cx);
        let view = cx.new(|cx| {
            observe_window_state(app_id, window, cx);
            build(&arguments, window, cx)
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(())
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
    boot_with_assets(default_assets(), title, width, height, build);
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
    boot_app_with_assets(app_id, default_assets(), title, width, height, build);
}

/// gpui-component's bundled fonts and icons — the default asset source every
/// `boot*` entry point falls back to when the app has none of its own.
fn default_assets() -> impl gpui::AssetSource {
    gpui_component_assets::Assets
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
    boot_app_window(app_id, assets, title, width, height, false, build);
}

/// [`boot_app_with_assets`] for an app with one window, as Mac Notes has:
/// a second launch brings the running app's window forward instead of
/// starting a second process, which could not own the app's menus (or, for
/// Notes, its library).
pub fn boot_single_window_app_with_assets<A, V, F>(
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
    if hand_off_to_running_instance(app_id, &[Vec::new()]) {
        focus_running_app(app_id);
        return;
    }
    boot_app_window(app_id, assets, title, width, height, true, build);
}

/// Bring `app_id`'s most recently used window forward through the
/// compositor, restoring it if it is minimised. The launch that handed off
/// to the running app does this: it holds the user's activation, which the
/// running process does not.
fn focus_running_app(app_id: &'static str) {
    #[cfg(target_os = "linux")]
    async_io::block_on(async {
        let snapshot = match rmac_compositor_niri::snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("{app_id}: could not find the running window: {error:?}");
                return;
            }
        };
        let Some(window) = snapshot
            .windows
            .iter()
            .filter(|window| window.app_id.as_deref() == Some(app_id))
            .max_by_key(|window| {
                window
                    .focus_timestamp
                    .as_ref()
                    .map(|stamp| (stamp.seconds, stamp.nanoseconds))
            })
            .map(|window| window.id)
        else {
            return;
        };
        let mut store = rmac_compositor::ParkingStore::load_default();
        let parked = store.entries().iter().any(|entry| entry.window == window);
        let actions = if parked {
            store.restore_actions(&[window])
        } else {
            vec![rmac_compositor::Action::FocusWindow { window }]
        };
        for action in &actions {
            if let Err(error) = rmac_compositor_niri::execute_action(action).await {
                eprintln!("{app_id}: could not bring its window forward: {error:?}");
            }
        }
        if parked {
            if let Err(error) = store.save_default() {
                eprintln!("could not save the parking set: {error}");
            }
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = app_id;
}

fn boot_app_window<A, V, F>(
    app_id: &'static str,
    assets: A,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    single_window: bool,
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
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            if single_window {
                // A later launch asks this process to come forward.
                crate::install_app_instance(
                    app_id,
                    |_, cx| {
                        if let Some((window, _)) = crate::menu_target::target(cx) {
                            window
                                .update(cx, |_, window, _| window.activate_window())
                                .ok();
                        }
                    },
                    cx,
                );
            } else {
                install_app_menu(app_id, cx);
            }
            // The caller names the visible window's size (the Mac's); the
            // outer bounds add the client frame around it.
            let (outer_width, outer_height) = outer_window_size(width, height);
            let mut options =
                window_options_for_app_with_title(app_id, title, outer_width, outer_height, cx);
            // The minimum size reaches the compositor as xdg_toplevel's
            // min_size, which is in window-geometry terms (without the
            // frame); in outer terms niri widens the window by the frame.
            options.window_min_size = Some(minimum_window_size(width, height));

            cx.open_window(options, move |window, cx| {
                reserve_client_frame(window);
                prepare_surface_window(window, cx);
                fit_to_display_after_first_frame(window, cx);
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

/// [`boot_app_instance_with_assets`] with gpui-component's default assets.
pub fn boot_app_instance<V, F>(
    app_id: &'static str,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    windows: Vec<Vec<String>>,
    build: F,
) where
    V: Render + 'static,
    F: Fn(&[String], &mut Window, &mut Context<V>) -> V + 'static,
{
    boot_app_instance_with_assets(
        app_id,
        default_assets(),
        title,
        width,
        height,
        windows,
        build,
    );
}

/// [`boot_app_with_assets`] for an app that keeps every window in one
/// process, as a macOS app does. `windows` holds one argument list per
/// window this launch asks for (none means one default window). If the app
/// is already running, this launch hands each list to it over D-Bus, the
/// running process opens a window for each, and this function returns
/// without starting GPUI. Otherwise it opens the windows itself, keeps
/// running with its menu after the last one closes (as the Mac's Dock
/// does), and serves later launches' requests with `build` too.
pub fn boot_app_instance_with_assets<A, V, F>(
    app_id: &'static str,
    assets: A,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    windows: Vec<Vec<String>>,
    build: F,
) where
    A: gpui::AssetSource,
    V: Render + 'static,
    F: Fn(&[String], &mut Window, &mut Context<V>) -> V + 'static,
{
    let mut windows = windows;
    if windows.is_empty() {
        windows.push(Vec::new());
    }
    if hand_off_to_running_instance(app_id, &windows) {
        return;
    }
    let fallback_title: SharedString = title.into();
    let title = rmac_apps::identity::window_title(app_id)
        .map(SharedString::from)
        .unwrap_or(fallback_title);
    let build = Rc::new(build);
    crate::application()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            init_application(cx);
            let requested = build.clone();
            let window_title = title.clone();
            let opener: OpenWindow = Rc::new(move |arguments, cx| {
                if let Err(error) = open_app_window(
                    app_id,
                    window_title.clone(),
                    width,
                    height,
                    arguments,
                    requested.clone(),
                    cx,
                ) {
                    eprintln!("{app_id} could not open a new window: {error}");
                }
            });
            cx.set_global(AppWindowOpener(opener.clone()));
            crate::runtime::install_app_instance(
                app_id,
                move |arguments, cx| opener(arguments, cx),
                cx,
            );
            let mut windows = windows.into_iter();
            if let Some(first) = windows.next() {
                open_app_window(
                    app_id,
                    title.clone(),
                    width,
                    height,
                    first,
                    build.clone(),
                    cx,
                )
                .expect("failed to open window");
            }
            for arguments in windows {
                if let Err(error) = open_app_window(
                    app_id,
                    title.clone(),
                    width,
                    height,
                    arguments,
                    build.clone(),
                    cx,
                ) {
                    eprintln!("{app_id} could not open another window: {error}");
                }
            }
            cx.activate(true);
        });
}

fn open_app_window<V, F>(
    app_id: &'static str,
    title: SharedString,
    width: f32,
    height: f32,
    arguments: Vec<String>,
    build: Rc<F>,
    cx: &mut App,
) -> gpui::Result<()>
where
    V: Render + 'static,
    F: Fn(&[String], &mut Window, &mut Context<V>) -> V + 'static,
{
    // The caller names the visible window's size (the Mac's); the outer
    // bounds add the client frame around it.
    let (outer_width, outer_height) = outer_window_size(width, height);
    let mut options =
        window_options_for_app_with_title(app_id, title, outer_width, outer_height, cx);
    options.window_min_size = Some(minimum_window_size(width, height));
    cx.open_window(options, move |window, cx| {
        reserve_client_frame(window);
        prepare_surface_window(window, cx);
        fit_to_display_after_first_frame(window, cx);
        let view = cx.new(|cx| {
            observe_window_state(app_id, window, cx);
            build(&arguments, window, cx)
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(())
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
    crate::application()
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
                fit_to_display_after_first_frame(window, cx);
                let view = cx.new(|cx| build(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}

#[cfg(test)]
mod fit_tests {
    use super::fit_to_screen;

    #[test]
    fn default_sizes_shrink_to_the_space_between_menu_bar_and_dock() {
        // The reference laptop is 1536 × 864 logical.
        assert_eq!(fit_to_screen(947.0, 833.0, 1536.0, 864.0), (947.0, 730.0));
        assert_eq!(fit_to_screen(700.0, 500.0, 1536.0, 864.0), (700.0, 500.0));
        assert_eq!(fit_to_screen(2000.0, 900.0, 1470.0, 956.0), (1430.0, 822.0));
    }
}
