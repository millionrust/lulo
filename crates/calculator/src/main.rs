//! rmac Calculator: macOS Calculator's Basic and Scientific modes.

// A GUI app on Windows: no console window behind it (ADR 0023).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod maths_notes;
mod view;

use std::borrow::Cow;

use gpui::{
    point, px, size, App, AppContext as _, AssetSource, Bounds, KeyBinding, Result, SharedString,
    WindowBackgroundAppearance, WindowBounds,
};
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
        ShowProgrammer,
        ShowConvert,
        ToggleRpnMode,
        ShowMathsNotes,
        ShowHistory,
        ToggleThousandsSeparator,
        DecimalPlaces0,
        DecimalPlaces1,
        DecimalPlaces2,
        DecimalPlaces3,
        DecimalPlaces4,
        DecimalPlaces5,
        DecimalPlaces6,
        DecimalPlaces7,
        DecimalPlaces8,
        DecimalPlaces9,
        DecimalPlaces10,
        DecimalPlaces11,
        DecimalPlaces12,
        DecimalPlaces13,
        DecimalPlaces14,
        DecimalPlaces15,
        EnterFullScreen,
        CloseWindow,
        QuitAndKeepWindows
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
            rmac_ui::bind_keys(
                cx,
                [
                    KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, Some("Calculator")),
                    KeyBinding::new(
                        rmac_ui::shortcuts::PASTE.keystroke,
                        Paste,
                        Some("Calculator"),
                    ),
                    KeyBinding::new("cmd-1", ShowBasic, Some("Calculator")),
                    KeyBinding::new("cmd-2", ShowScientific, Some("Calculator")),
                    KeyBinding::new("cmd-3", ShowProgrammer, Some("Calculator")),
                    KeyBinding::new("alt-cmd-c", ShowConvert, Some("Calculator")),
                    KeyBinding::new("cmd-r", ToggleRpnMode, Some("Calculator")),
                    KeyBinding::new("alt-cmd-m", ShowMathsNotes, Some("Calculator")),
                    KeyBinding::new("ctrl-cmd-s", ShowHistory, Some("Calculator")),
                    KeyBinding::new(
                        rmac_ui::shortcuts::CLOSE.keystroke,
                        CloseWindow,
                        Some("Calculator"),
                    ),
                    KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, Some("Calculator")),
                    KeyBinding::new("alt-cmd-q", QuitAndKeepWindows, Some("Calculator")),
                ],
            );
            // Application ▸ Quit and Keep Windows (⌥⌘Q): Calculator has a
            // single window and no document state to restore (unlike
            // Preview's open-file list), so this is the same quit as ⌘Q.
            cx.on_action(|_: &QuitAndKeepWindows, cx| cx.quit());
            rmac_ui::install_app_menu(CALCULATOR, cx);
            // Basic is the starting mode, so View ▸ Basic is the ticked one.
            rmac_ui::set_menu_checked("calculator::ShowBasic", true, cx);
            rmac_ui::set_menu_checked("calculator::DecimalPlaces8", true, cx);
            // RPN Mode only drives Basic's keypad (see `rpn.rs`'s module doc
            // comment); it starts unchecked and enabled, since Basic is the
            // starting mode.
            rmac_ui::set_menu_checked("calculator::ToggleRpnMode", false, cx);
            // Calculator's fixed Basic and Scientific surfaces cannot enter
            // full screen on the Mac; the View row is present but greyed.
            rmac_ui::set_menu_enabled("calculator::EnterFullScreen", false, cx);

            // Calculator is fixed-size like on macOS: keep a restored position
            // but never a restored size.
            // On Windows the window also holds its menu strip above the
            // calculator (ADR 0023); elsewhere that height is zero.
            let fixed = size(
                px(WINDOW_WIDTH),
                px(WINDOW_HEIGHT + rmac_ui::menu_strip_height(cx)),
            );
            // A fixed-size calculator has no resize edge. Its own rounded
            // surface fills the exact compositor bounds, without the 12 pt
            // client shadow frame used by resizable app windows.
            let mut options =
                rmac_ui::window_options_for_app(CALCULATOR, WINDOW_WIDTH, WINDOW_HEIGHT, cx);
            let origin = options
                .window_bounds
                .map(|bounds| bounds.get_bounds().origin)
                .unwrap_or_else(|| point(px(0.0), px(0.0)));
            options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(origin, fixed)));
            options.window_min_size = Some(fixed);
            options.is_resizable = false;
            options.window_background = WindowBackgroundAppearance::Transparent;

            cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(CALCULATOR, window, cx);
                    CalculatorView::new(cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| rmac_ui::fixed_surface_root(view, window, cx))
            })
            .expect("failed to open the Calculator window");
            cx.activate(true);
        });
}
