//! rmac Weather: current conditions, the hourly strip and ten days ahead for
//! the cities the user adds. No location lookup; data from Open-Meteo.

// A GUI app on Windows: no console window behind it (ADR 0023).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod settings_window;
mod view;

use std::borrow::Cow;

use gpui::{px, size, App, AppContext as _, AssetSource, KeyBinding, Result, SharedString};
use gpui_component::Root;
use rmac_ui::app_id::WEATHER;
use rmac_weather::metrics;

use crate::view::WeatherView;

gpui::actions!(
    weather,
    [
        Refresh,
        FindCity,
        UseCelsius,
        UseFahrenheit,
        ToggleSidebar,
        AddLocationToList,
        ToggleFullScreen,
        ShowSettings,
        CloseWindow,
        QuitAndKeepWindows
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct WeatherAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = WeatherAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = WeatherAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

fn main() {
    // One process per app, as on the Mac: a later launch while Weather is
    // already running brings its window forward instead of opening a
    // second one.
    if rmac_ui::hand_off_to_running_instance(WEATHER, &[Vec::new()]) {
        rmac_ui::focus_running_app(WEATHER);
        return;
    }
    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            let context = Some("Weather");
            rmac_ui::bind_keys(
                cx,
                [
                    KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, FindCity, context),
                    KeyBinding::new(rmac_ui::shortcuts::CLOSE.keystroke, CloseWindow, context),
                    KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, context),
                    KeyBinding::new("ctrl-cmd-s", ToggleSidebar, context),
                    KeyBinding::new("shift-cmd-l", AddLocationToList, context),
                    KeyBinding::new("cmd-,", ShowSettings, context),
                    KeyBinding::new("alt-cmd-q", QuitAndKeepWindows, context),
                ],
            );
            // Application ▸ Quit and Keep Windows (⌥⌘Q): Weather has no
            // per-window document state to restore (unlike Preview's
            // open-file list — its forecast locations are saved
            // continuously as settings, not on quit), so this is the same
            // quit as ⌘Q.
            cx.on_action(|_: &QuitAndKeepWindows, cx| cx.quit());
            rmac_ui::install_app_instance(WEATHER, |_, cx| rmac_ui::activate_app_window(cx), cx);
            let (width, height) = metrics::WINDOW;
            let mut options = rmac_ui::window_options_for_app(WEATHER, width, height, cx);
            options.window_min_size =
                Some(size(px(metrics::MIN_WINDOW.0), px(metrics::MIN_WINDOW.1)));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                rmac_ui::fit_to_display_after_first_frame(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(WEATHER, window, cx);
                    WeatherView::new(window, cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("rmac-weather: could not open a window: {error}");
                cx.quit();
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}
