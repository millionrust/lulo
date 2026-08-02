use std::{env, fs};

use gpui::{px, App, AppContext as _, Window};

use crate::{components, theme};

const BENCHMARK_READY_FILE_ENV: &str = "RMAC_BENCHMARK_READY_FILE";
const BASE_REM_SIZE: f32 = 16.0;

fn apply_window_text_scale(window: &mut Window, scale: rmac_appearance::TextScale) {
    window.set_rem_size(px(BASE_REM_SIZE * scale.factor()));
}

/// Initialize the shared component, theme, and accessibility runtimes for a
/// long-lived shell process that creates windows on demand.
pub fn init_application(cx: &mut App) {
    gpui_component::init(cx);
    components::init(cx);
    start_theme_runtime(cx);
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
