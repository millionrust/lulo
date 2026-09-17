use std::{env, fs};

use gpui::{px, App, AppContext as _, SharedString, Window};

use crate::{components, theme};

const BENCHMARK_READY_FILE_ENV: &str = "RMAC_BENCHMARK_READY_FILE";
const COLOR_SCHEME_ENV: &str = "RMAC_COLOR_SCHEME";
const BASE_REM_SIZE: f32 = 16.0;

fn apply_window_text_scale(window: &mut Window, scale: rmac_appearance::TextScale) {
    window.set_rem_size(px(BASE_REM_SIZE * scale.factor()));
}

/// Initialize the shared component, theme, and accessibility runtimes for a
/// long-lived shell process that creates windows on demand.
pub fn init_application(cx: &mut App) {
    seed_initial_theme();
    warn_if_ui_font_missing(cx);
    gpui_component::init(cx);
    components::init(cx);
    apply_component_theme(cx);
    start_theme_runtime(cx);
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
    theme.input = colors.separator.hsla();
    theme.accent = colors.selection_unfocused.hsla();
    theme.accent_foreground = colors.text.hsla();
    theme.secondary = colors.control_fill.hsla();
    theme.secondary_foreground = colors.text.hsla();
    theme.secondary_hover = colors.control_fill_hover.hsla();
    theme.secondary_active = colors.hover.hsla();
    theme.muted = colors.control_fill.hsla();
    theme.muted_foreground = colors.text_secondary.hsla();
    theme.popover = colors.raised.hsla();
    theme.popover_foreground = colors.text.hsla();
    theme.list = colors.window.hsla();
    theme.list_hover = colors.hover.hsla();
    theme.list_active = colors.accent.hsla();
    theme.list_active_border = colors.accent.hsla();
    theme.list_even = colors.row_alternate.hsla();
    theme.list_head = colors.chrome.hsla();
    theme.selection = colors.accent_subtle.hsla();
    theme.caret = colors.accent.hsla();
    theme.ring = colors.accent.hsla();
    theme.danger = colors.danger.hsla();
    theme.danger_foreground = colors.on_danger.hsla();
    theme.danger_hover = colors.danger.hsla();
    theme.danger_active = colors.danger.hsla();
    theme.sidebar = colors.sidebar.hsla();
    theme.sidebar_accent = colors.accent.hsla();
    theme.sidebar_accent_foreground = colors.on_accent.hsla();
    theme.sidebar_border = colors.separator.hsla();
    theme.scrollbar_thumb = colors.separator.hsla();
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
/// from the desktop menu bar into the currently active GPUI window.
pub fn install_app_menu(app_id: &'static str, cx: &mut App) {
    #[cfg(target_os = "linux")]
    {
        let Some(menus) = rmac_app_menu::definition(app_id, cx.all_action_names()) else {
            return;
        };
        let (activation_tx, activation_rx) = rmac_app_menu::activation_channel();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_app_menu::serve(app_id, menus, activation_tx).await {
                    eprintln!("{app_id} menu export stopped: {error}");
                }
            })
            .detach();
        cx.spawn(async move |cx| {
            while let Ok(action_name) = activation_rx.recv().await {
                let _ = cx.update(|cx| match cx.build_action(&action_name, None) {
                    Ok(action) => cx.dispatch_action(action.as_ref()),
                    Err(error) => eprintln!("ignored unavailable {app_id} menu action: {error}"),
                });
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (app_id, cx);
}

/// Apply the current shared theme and text scale before an on-demand shell
/// surface renders its first frame.
pub fn prepare_surface_window(window: &mut Window, cx: &mut App) {
    apply_window_text_scale(window, theme::current().text_scale);
    gpui_component::theme::Theme::change(current_component_theme_mode(), Some(window), cx);
    mark_benchmark_first_frame(window);
}

/// Scales an application-owned text size with the live rmac accessibility
/// preference. Layout dimensions remain independent logical pixels.
pub fn text_px(base: f32) -> gpui::Pixels {
    px(base * theme::current().text_scale.factor())
}

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
}
