//! rmac Weather: current conditions, the hourly strip and ten days ahead for
//! the cities the user adds. No location lookup; data from Open-Meteo.

mod view;

use std::borrow::Cow;

use gpui::{px, size, App, AppContext as _, AssetSource, KeyBinding, Result, SharedString};
use gpui_component::Root;
use rmac_ui::app_id::WEATHER;
use rmac_weather::metrics;

use crate::view::WeatherView;

gpui::actions!(
    weather,
    [Refresh, FindCity, UseCelsius, UseFahrenheit, CloseWindow]
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
    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            let context = Some("Weather");
            cx.bind_keys([
                KeyBinding::new("cmd-r", Refresh, context),
                KeyBinding::new(rmac_ui::shortcuts::FIND.keystroke, FindCity, context),
                KeyBinding::new(rmac_ui::shortcuts::CLOSE.keystroke, CloseWindow, context),
            ]);
            rmac_ui::install_app_menu(WEATHER, cx);
            let (width, height) = metrics::WINDOW;
            let mut options = rmac_ui::window_options_for_app(WEATHER, width, height, cx);
            options.window_min_size =
                Some(size(px(metrics::MIN_WINDOW.0), px(metrics::MIN_WINDOW.1)));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
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
