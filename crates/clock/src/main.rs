//! rmac Clock: macOS 26 Clock's World Clock, Alarms, Stopwatch and Timers.
//!
//! `rmac-clock --ring-due` is the headless ring process the user systemd
//! timer starts; it never opens a window.

mod ring;
mod view;

use std::borrow::Cow;

use gpui::{App, AppContext as _, AssetSource, KeyBinding, Result, SharedString};
use gpui_component::Root;
use rmac_clock::metrics;
use rmac_ui::app_id::CLOCK;

use crate::view::ClockView;

gpui::actions!(
    clock,
    [
        ShowWorldClock,
        ShowAlarms,
        ShowStopwatch,
        ShowTimers,
        NewItem,
        StartStop,
        LapReset,
        CloseWindow,
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct ClockAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = ClockAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = ClockAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

fn bind_keys(cx: &mut App) {
    let context = Some("Clock");
    cx.bind_keys([
        KeyBinding::new("cmd-1", ShowWorldClock, context),
        KeyBinding::new("cmd-2", ShowAlarms, context),
        KeyBinding::new("cmd-3", ShowStopwatch, context),
        KeyBinding::new("cmd-4", ShowTimers, context),
        KeyBinding::new(rmac_ui::shortcuts::NEW.keystroke, NewItem, context),
        KeyBinding::new(rmac_ui::shortcuts::CLOSE.keystroke, CloseWindow, context),
    ]);
}

fn main() -> std::process::ExitCode {
    if std::env::args().any(|argument| argument == rmac_clock::schedule::RING_ARGUMENT) {
        return ring::run();
    }
    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            bind_keys(cx);
            rmac_ui::install_app_menu(CLOCK, cx);
            let (width, height) = metrics::WINDOW;
            let mut options = rmac_ui::window_options_for_app(CLOCK, width, height, cx);
            options.window_min_size = Some(gpui::size(
                gpui::px(metrics::MIN_WINDOW.0),
                gpui::px(metrics::MIN_WINDOW.1),
            ));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(CLOCK, window, cx);
                    ClockView::new(window, cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("rmac-clock: could not open a window: {error}");
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
    std::process::ExitCode::SUCCESS
}
