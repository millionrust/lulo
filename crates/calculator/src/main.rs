//! rmac Calculator: macOS Calculator's Basic and Scientific modes.

mod view;

use std::borrow::Cow;

use gpui::{
    point, px, size, App, AppContext as _, AssetSource, Bounds, KeyBinding, Result, SharedString,
    WindowBounds,
};
use gpui_component::Root;
use rmac_calculator::keypad::{WINDOW_HEIGHT, WINDOW_WIDTH};
use rmac_ui::app_id::CALCULATOR;

use crate::view::CalculatorView;

gpui::actions!(
    calculator,
    [
        Copy,
        Paste,
        ShowBasic,
        ShowScientific,
        ShowHistory,
        CloseWindow
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct CalculatorAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = CalculatorAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = CalculatorAssets::iter()
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
            cx.bind_keys([
                KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, Some("Calculator")),
                KeyBinding::new(
                    rmac_ui::shortcuts::PASTE.keystroke,
                    Paste,
                    Some("Calculator"),
                ),
                KeyBinding::new("cmd-1", ShowBasic, Some("Calculator")),
                KeyBinding::new("cmd-2", ShowScientific, Some("Calculator")),
                KeyBinding::new("ctrl-cmd-s", ShowHistory, Some("Calculator")),
                KeyBinding::new(
                    rmac_ui::shortcuts::CLOSE.keystroke,
                    CloseWindow,
                    Some("Calculator"),
                ),
            ]);
            rmac_ui::install_app_menu(CALCULATOR, cx);

            // Calculator is fixed-size like on macOS: keep a restored position
            // but never a restored size.
            let mut options =
                rmac_ui::window_options_for_app(CALCULATOR, WINDOW_WIDTH, WINDOW_HEIGHT, cx);
            let fixed = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
            let origin = options
                .window_bounds
                .map(|bounds| bounds.get_bounds().origin)
                .unwrap_or_else(|| point(px(0.0), px(0.0)));
            options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(origin, fixed)));
            options.window_min_size = Some(fixed);
            options.is_resizable = false;

            cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(CALCULATOR, window, cx);
                    CalculatorView::new(cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open the Calculator window");
            cx.activate(true);
        });
}
