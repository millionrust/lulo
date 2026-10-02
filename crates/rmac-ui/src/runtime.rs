use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{env, fs};

use gpui::{px, AnyView, App, AppContext as _, Context, SharedString, Styled as _, Window};

use crate::{components, text_keys, theme};

const BENCHMARK_READY_FILE_ENV: &str = "RMAC_BENCHMARK_READY_FILE";

/// Set by [`defer_content_ready`]: while true, the generic first-frame
/// benchmark marker (written from every `prepare_surface_window` call) stays
/// silent, so only an explicit [`mark_content_ready`] call from the app can
/// write the benchmark-ready file. Each rmac shell process hosts at most one
/// benchmarked app window, so a single process-wide flag is enough.
static DEFER_CONTENT_READY: AtomicBool = AtomicBool::new(false);
/// Guards the benchmark-ready file so only the first qualifying frame writes
/// it, whichever of `mark_benchmark_first_frame` / `mark_content_ready` gets
/// there first.
static CONTENT_READY_WRITTEN: AtomicBool = AtomicBool::new(false);
const COLOR_SCHEME_ENV: &str = "RMAC_COLOR_SCHEME";
// GPUI defaults to a web-style 16 px rem. macOS desktop body copy is 13 px;
// keeping 16 here made every unqualified component label look oversized even
// though the semantic type scale was already correct.
const BASE_REM_SIZE: f32 = 13.0;

fn apply_window_text_scale(window: &mut Window, scale: rmac_appearance::TextScale) {
    window.set_rem_size(px(BASE_REM_SIZE * scale.factor()));
}

/// Initialize the shared component, theme, and accessibility runtimes for a
/// long-lived shell process that creates windows on demand.
pub fn init_application(cx: &mut App) {
    seed_initial_theme();
    warn_if_ui_font_missing(cx);
    gpui_component::init(cx);
    text_keys::init(cx);
    components::init(cx);
    apply_component_theme(cx);
    start_theme_runtime(cx);
    crate::session::install(cx);
}

/// Release an on-demand shell renderer after its last window has been closed
/// for the configured interval. A new close invalidates the previous timer;
/// no periodic wake-up is needed while the surface is visible or idle.
pub fn install_surface_idle_exit(cx: &mut App) {
    let seconds = env::var("RMAC_SURFACE_IDLE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1800)
        .clamp(1, 86400);
    let interval = Duration::from_secs(seconds);
    let generation = Arc::new(AtomicU64::new(0));
    cx.on_window_closed(move |cx, _| {
        let current = generation.fetch_add(1, Ordering::AcqRel) + 1;
        schedule_surface_idle_exit(cx, generation.clone(), current, interval);
    })
    .detach();
}

fn schedule_surface_idle_exit(
    cx: &mut App,
    generation: Arc<AtomicU64>,
    current: u64,
    interval: Duration,
) {
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        cx.background_executor().timer(interval).await;
        if generation.load(Ordering::Acquire) == current {
            cx.update(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            });
        }
    })
    .detach();
}

/// Push the resolved rmac tokens into gpui-component's global theme so shared
/// component primitives (inputs, sliders, menus, tables) share the same colors,
/// radii, and fonts as the rmac-owned controls.
fn apply_component_theme(cx: &mut App) {
    let tokens = theme::current();
    let colors = tokens.colors;
    let theme = gpui_component::theme::Theme::global_mut(cx);
    theme.font_family = SharedString::from(crate::UI_FONT);
    theme.mono_font_family = SharedString::from(crate::MONO_FONT);
    theme.radius = px(tokens.radii.control);
    theme.radius_lg = px(tokens.radii.popover);

    theme.primary = colors.accent.hsla();
    theme.primary_foreground = colors.on_accent.hsla();
    theme.primary_hover = colors.accent.hsla();
    theme.primary_active = colors.accent.hsla();
    theme.background = colors.window.hsla();
    theme.foreground = colors.text.hsla();
    theme.border = colors.separator.hsla();
    theme.input = colors.field_fill.hsla();
    theme.accent = colors.selection_unfocused.hsla();
    theme.accent_foreground = colors.text.hsla();
    theme.secondary = colors.button_secondary.hsla();
    theme.secondary_foreground = colors.text.hsla();
    theme.secondary_hover = colors.control_fill_hover.hsla();
    theme.secondary_active = colors.hover.hsla();
    theme.muted = colors.control_fill.hsla();
    theme.muted_foreground = colors.text_secondary.hsla();
    theme.popover = colors.raised.hsla();
    theme.popover_foreground = colors.text.hsla();
    theme.list_hover = colors.hover.hsla();
    theme.list_active = colors.accent.hsla();
    theme.list_active_border = colors.accent.hsla();
    theme.list_even = colors.row_alternate.hsla();
    theme.list_head = colors.chrome.hsla();
    // Text views take AppKit's measured insertion point and selected-text
    // highlight rather than the accent fills used by lists and buttons.
    theme.selection = crate::mac::text_selection();
    theme.caret = crate::mac::text_caret();
    theme.ring = colors.accent.hsla();
    theme.danger = colors.button_destructive.hsla();
    theme.danger_foreground = colors.on_button_destructive.hsla();
    theme.danger_hover = colors.button_destructive.hsla();
    theme.danger_active = colors.button_destructive.hsla();
    theme.sidebar = colors.sidebar.hsla();
    theme.sidebar_accent = colors.accent.hsla();
    theme.sidebar_accent_foreground = colors.on_accent.hsla();
    theme.sidebar_border = colors.separator.hsla();
    // The overlay scroller's knob. gpui-component paints scroll bars from
    // its derived tokens, which `Theme::change` rebuilds from its own
    // defaults, so set the tokens as well as the colours.
    theme.scrollbar_thumb = crate::mac::scroller_knob();
    theme.scrollbar_thumb_hover = crate::mac::scroller_knob_hover();
    theme.tokens.scrollbar_thumb = crate::mac::scroller_knob().into();
    theme.tokens.scrollbar_thumb_hover = crate::mac::scroller_knob_hover().into();
    theme.group_box = colors.raised.hsla();
    theme.group_box_foreground = colors.text.hsla();
    theme.progress_bar = colors.control_fill.hsla();
    theme.sidebar_foreground = colors.text_secondary.hsla();
    theme.sidebar_primary = colors.accent.hsla();
    theme.sidebar_primary_foreground = colors.on_accent.hsla();
    theme.switch = colors.control_fill.hsla();
    theme.switch_thumb = colors.white.hsla();
    theme.slider_bar = colors.control_fill.hsla();
    theme.slider_thumb = colors.white.hsla();
    theme.tab = colors.window.hsla();
    theme.tab_active = colors.accent.hsla();
    theme.tab_active_foreground = colors.on_accent.hsla();
    theme.tab_bar = colors.window.hsla();
    theme.tab_bar_segmented = colors.control_fill.hsla();
    theme.tab_foreground = colors.text_secondary.hsla();
    theme.table = colors.window.hsla();
    theme.table_active = colors.accent.hsla();
    theme.table_active_border = colors.accent.hsla();
    theme.table_even = colors.row_alternate.hsla();
    theme.table_head = colors.chrome.hsla();
    theme.table_head_foreground = colors.text_secondary.hsla();
    theme.table_hover = colors.hover.hsla();
    theme.table_row_border = colors.separator.hsla();
    theme.title_bar = colors.chrome.hsla();
    theme.title_bar_border = colors.separator.hsla();
    theme.overlay = colors.scrim.hsla();
    theme.drag_border = colors.accent.hsla();
    theme.drop_target = colors.selection_unfocused.hsla();
    theme.skeleton = colors.control_fill.hsla();
    theme.success = colors.system_green.hsla();
    theme.success_foreground = colors.white.hsla();
    theme.warning = colors.warning_background.hsla();
    theme.warning_foreground = colors.warning_text.hsla();
    theme.info = colors.system_blue.hsla();
    theme.red = colors.system_red.hsla();
    theme.green = colors.system_green.hsla();
    theme.blue = colors.system_blue.hsla();
    theme.yellow = colors.system_yellow.hsla();
    theme.magenta = colors.system_pink.hsla();
    theme.cyan = colors.system_teal.hsla();
}

/// Warn once if the declared UI font is absent. GPUI silently falls back to
/// the system sans, so this keeps a missing `fonts-inter` package diagnosable
/// without ever blocking startup.
fn warn_if_ui_font_missing(cx: &App) {
    if cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|name| name == crate::UI_FONT)
    {
        return;
    }
    eprintln!(
        "rmac: UI font '{}' is not installed; using the system sans instead. Install the 'fonts-inter' package for the intended look.",
        crate::UI_FONT
    );
}

/// Resolve the session's exported host scheme and any explicit rmac preference
/// before the first window is painted. The portal remains authoritative after
/// startup, but it must not make dark sessions flash a light first frame.
fn seed_initial_theme() {
    let Some(host) = initial_host_appearance() else {
        return;
    };
    if let Ok(tokens) = load_tokens_with_host(host) {
        let _ = theme::set_current(tokens);
    }
}

fn initial_host_appearance() -> Option<rmac_appearance::Snapshot> {
    let value = env::var(COLOR_SCHEME_ENV).ok()?;
    initial_host_appearance_from(&value)
}

fn initial_host_appearance_from(value: &str) -> Option<rmac_appearance::Snapshot> {
    let color_scheme = match value {
        "dark" => rmac_appearance::ColorScheme::PreferDark,
        "light" => rmac_appearance::ColorScheme::PreferLight,
        _ => return None,
    };
    Some(rmac_appearance::Snapshot {
        available: true,
        color_scheme,
        capabilities: rmac_appearance::Capabilities {
            color_scheme: true,
            ..rmac_appearance::Capabilities::default()
        },
        ..rmac_appearance::Snapshot::default()
    })
}

/// Publish this first-party app's registered commands and route activations
/// from the desktop menu bar into the app's key window (see
/// [`crate::register_menu_target`]).
pub fn install_app_menu(app_id: &'static str, cx: &mut App) {
    crate::app_menu::install(app_id, None, cx);
}

/// [`install_app_menu`] for an app whose windows share one process: later
/// launches hand their arguments to `open_window` here instead of starting a
/// second process that could not own the app's menu name. Pair it with
/// [`crate::hand_off_to_running_instance`] before GPUI starts.
pub fn install_app_instance(
    app_id: &'static str,
    open_window: impl Fn(Vec<String>, &mut App) + 'static,
    cx: &mut App,
) {
    crate::app_menu::install(app_id, Some(Box::new(open_window)), cx);
}

/// Apply the current shared theme and text scale before an on-demand shell
/// surface renders its first frame.
pub fn prepare_surface_window(window: &mut Window, cx: &mut App) {
    apply_window_text_scale(window, theme::current().text_scale);
    gpui_component::theme::Theme::change(current_component_theme_mode(), Some(window), cx);
    // `Theme::change` reloads gpui-component's stock palette; put the rmac
    // tokens back so inputs, carets, selections and scroll bars in this
    // window draw with them (as `apply_resolved_tokens` does on a change).
    apply_component_theme(cx);
    mark_benchmark_first_frame(window);
}

/// Root for a shell surface such as Spotlight or Control Center. The surface
/// draws its own rounded material, so the root must stay clear: the default
/// root paints the opaque window background and, on Linux, a square
/// client-side window border and shadow over the whole layer surface.
pub fn shell_surface_root(
    view: impl Into<AnyView>,
    window: &mut Window,
    cx: &mut Context<gpui_component::Root>,
) -> gpui_component::Root {
    gpui_component::Root::new(view, window, cx)
        .bordered(false)
        .bg(gpui::transparent_black())
}

/// A transparent, keyboard-inert layer-shell surface that fills the rest of
/// the display below `reserved_top` and runs `on_click` — removing itself
/// first — the instant a pointer button goes down on it.
///
/// Control Center and the Notification Center panel each own only the small
/// rectangle they draw; outside that rectangle a click reaches whatever is
/// physically there instead (the Dock's own surface never requests keyboard
/// focus at all, so it never tells the popover to close, and even a surface
/// that does take focus on click only does so if the compositor transfers
/// Wayland keyboard focus, which this does not depend on). This closes the
/// gap by catching the pointer press itself: `Layer::Overlay` sits above the
/// Dock's `Layer::Top` and above ordinary windows unconditionally (the
/// wlr-layer-shell stacking order is fixed, not creation-order-dependent),
/// and `reserved_top` excludes the shared top-bar band
/// (`shell/bins/rmac-menubar/src/main.rs`'s `MENU_SURFACE_HEIGHT`) so this
/// never competes with the menu bar's own on-demand surface for a click
/// meant to switch menus or dismiss a different popover there.
///
/// Returns `None` (opening nothing) when the display is shorter than
/// `reserved_top`, or if the window fails to open.
#[cfg(target_os = "linux")]
pub fn open_outside_click_catcher(
    namespace: &str,
    display: std::rc::Rc<dyn gpui::PlatformDisplay>,
    reserved_top: gpui::Pixels,
    on_click: impl Fn(&mut App) + 'static,
    cx: &mut App,
) -> Option<gpui::AnyWindowHandle> {
    open_outside_click_catcher_around(namespace, display, reserved_top, None, on_click, cx)
}

/// Like [`open_outside_click_catcher`], but leaves `excluded` click-through.
/// The bounds are in display coordinates and normally enclose the popover
/// itself. Four input rectangles cover the rest of the display without
/// stealing pointer events from controls inside the popover.
#[cfg(target_os = "linux")]
pub fn open_outside_click_catcher_around(
    namespace: &str,
    display: std::rc::Rc<dyn gpui::PlatformDisplay>,
    reserved_top: gpui::Pixels,
    excluded: Option<gpui::Bounds<gpui::Pixels>>,
    on_click: impl Fn(&mut App) + 'static,
    cx: &mut App,
) -> Option<gpui::AnyWindowHandle> {
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    use gpui::{
        point, size, AnyWindowHandle, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
    };

    let bounds = display.bounds();
    let height = bounds.size.height - reserved_top;
    if height <= gpui::px(0.0) {
        return None;
    }
    let on_click = std::rc::Rc::new(on_click);
    let input_regions = excluded.map(|excluded| {
        let width = f32::from(bounds.size.width);
        let height = f32::from(height);
        let left = f32::from(excluded.origin.x - bounds.origin.x).clamp(0.0, width);
        let top = f32::from(excluded.origin.y - bounds.origin.y - reserved_top).clamp(0.0, height);
        let right = (left + f32::from(excluded.size.width)).clamp(left, width);
        let bottom = (top + f32::from(excluded.size.height)).clamp(top, height);
        let region = |x: f32, y: f32, w: f32, h: f32| {
            Bounds::new(
                point(gpui::px(x), gpui::px(y)),
                size(gpui::px(w), gpui::px(h)),
            )
        };
        [
            region(0.0, 0.0, width, top),
            region(0.0, bottom, width, height - bottom),
            region(0.0, top, left, bottom - top),
            region(right, top, width - right, bottom - top),
        ]
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(gpui::px(0.0), gpui::px(0.0)),
            size: size(bounds.size.width, height),
        })),
        titlebar: None,
        focus: false,
        show: true,
        display_id: Some(display.id()),
        app_id: Some("dev.rmac.OutsideClickCatcher".to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: namespace.to_owned(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM,
            margin: Some((reserved_top, gpui::px(0.0), gpui::px(0.0), gpui::px(0.0))),
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match cx.open_window(options, move |_, cx| {
        let left = on_click.clone();
        let right = on_click.clone();
        cx.new(|_| OutsideClickCatcher {
            left,
            right,
            input_regions,
        })
    }) {
        Ok(handle) => Some(AnyWindowHandle::from(handle)),
        Err(error) => {
            eprintln!("outside click catcher {namespace} failed to open: {error}");
            None
        }
    }
}

#[cfg(target_os = "linux")]
struct OutsideClickCatcher {
    left: std::rc::Rc<dyn Fn(&mut App)>,
    right: std::rc::Rc<dyn Fn(&mut App)>,
    input_regions: Option<[gpui::Bounds<gpui::Pixels>; 4]>,
}

#[cfg(target_os = "linux")]
impl gpui::Render for OutsideClickCatcher {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{InteractiveElement, MouseButton};

        if let Some(regions) = &self.input_regions {
            window.set_input_region(Some(regions));
        }

        let left = self.left.clone();
        let right = self.right.clone();
        gpui::div()
            .size_full()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.remove_window();
                left(cx);
            })
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                window.remove_window();
                right(cx);
            })
    }
}

/// Scales an application-owned text size with the live rmac accessibility
/// preference. Layout dimensions remain independent logical pixels.
pub fn text_px(base: f32) -> gpui::Pixels {
    px(base * theme::current().text_scale.factor())
}

/// Opt in to explicit content-readiness signalling for the performance
/// harness. Call this once, before creating the app's first window, from an
/// app whose first GPUI frame does not yet show its real content (for
/// example, Notes paints a "Starting" placeholder before its library worker
/// replies). Until the app calls [`mark_content_ready`], the generic
/// first-frame marker written by every `prepare_surface_window` call stays
/// silent, so the benchmark harness times launch-to-interactive rather than
/// launch-to-first-frame.
///
/// Apps that never call this (including the fallback for any app that
/// forgets to) keep the original behavior: the harness treats the first
/// frame as "ready", which is correct for an app whose first frame already
/// is its real content (for example, Text Editor's document is loaded
/// before its window is created).
pub fn defer_content_ready() {
    DEFER_CONTENT_READY.store(true, Ordering::SeqCst);
}

/// Marks that this window's *real* content — not a loading placeholder — is
/// on screen. Safe to call on every render pass: only the first call after
/// [`defer_content_ready`] writes the benchmark-ready file, and calls before
/// `defer_content_ready` runs (or from apps that never call it) are no-ops
/// beyond the generic first-frame marker already covering them.
pub fn mark_content_ready(window: &Window) {
    write_benchmark_marker_once(window);
}

/// Writes an opt-in marker after the window completes its first frame,
/// unless the app has deferred that signal to an explicit
/// [`mark_content_ready`] call.
///
/// The performance harness sets the environment variable. Normal application
/// launches do not set it and perform no filesystem I/O.
fn mark_benchmark_first_frame(window: &Window) {
    if DEFER_CONTENT_READY.load(Ordering::SeqCst) {
        return;
    }
    write_benchmark_marker_once(window);
}

/// Writes the benchmark-ready file the first time this is called for the
/// process, scheduled after the current frame finishes presenting. Later
/// calls (from either the generic first-frame path or an app's explicit
/// [`mark_content_ready`]) are no-ops.
fn write_benchmark_marker_once(window: &Window) {
    if CONTENT_READY_WRITTEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(path) = env::var_os(BENCHMARK_READY_FILE_ENV) else {
        return;
    };

    window.on_next_frame(move |_, _| {
        fs::write(&path, b"ready\n")
            .unwrap_or_else(|error| panic!("write benchmark marker {path:?}: {error}"));
    });
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

    // The wallpaper renderer publishes a bounded per-output colour snapshot.
    // Re-resolve opaque app surfaces whenever that authority changes.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let watcher = cx
            .background_executor()
            .spawn(async {
                rmac_theme::WallpaperColorStore::from_environment()
                    .and_then(|store| store.watch())
                    .map_err(|error| error.to_string())
            })
            .await;
        let Ok(watcher) = watcher else {
            return;
        };
        while watcher.recv().await.is_ok() {
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
    let wallpaper_tint = if resolved.preferences.allow_wallpaper_tinting {
        rmac_theme::WallpaperColorStore::from_environment()
            .and_then(|store| store.load())
            .ok()
            .and_then(|colors| colors.primary())
            .map(|color| color.dominant)
    } else {
        None
    };
    let mut tokens =
        theme::ThemeTokens::from_appearance_with_wallpaper(resolved.effective, wallpaper_tint);
    // Text highlight is a direct preference pass-through, not a host-resolved
    // value, so it is applied here rather than threaded through
    // `ResolvedAppearance`.
    tokens.text_highlight = resolved.preferences.text_highlight;
    Ok(tokens)
}

fn apply_resolved_tokens(tokens: theme::ThemeTokens, cx: &mut gpui::AsyncApp) {
    if !theme::set_current(tokens) {
        return;
    }
    let mode = component_theme_mode(tokens.color_scheme);
    cx.update(|app| {
        let windows = app.windows();
        for handle in windows {
            let _ = app.update_window(handle, |_, window, _| {
                apply_window_text_scale(window, tokens.text_scale);
            });
        }
        gpui_component::theme::Theme::change(mode, None, app);
        apply_component_theme(app);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_host_scheme_accepts_only_session_owned_values() {
        assert_eq!(
            initial_host_appearance_from("dark").unwrap().color_scheme,
            rmac_appearance::ColorScheme::PreferDark
        );
        assert_eq!(
            initial_host_appearance_from("light").unwrap().color_scheme,
            rmac_appearance::ColorScheme::PreferLight
        );
        assert!(initial_host_appearance_from("unknown").is_none());
    }

    #[test]
    fn unqualified_component_copy_uses_the_body_scale() {
        assert_eq!(BASE_REM_SIZE, 13.0);
        assert_eq!(theme::ThemeTokens::light_default().typography.body, 13.0);
    }
}
