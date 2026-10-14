//! rmac Media Player: QuickTime Player's minimal window for audio and video,
//! played by libmpv and published over MPRIS.

#[cfg(target_os = "linux")]
mod mpris;
mod mpv;
mod view;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::{px, size, App, AppContext as _, AssetSource, KeyBinding, Result, SharedString};
use gpui_component::Root;
use rmac_player::metrics;
use rmac_player::playlist::{self, Kind, Playlist};
use rmac_ui::app_id::PLAYER;

use crate::view::PlayerView;

gpui::actions!(
    player,
    [
        OpenFile,
        CloseWindow,
        PlayPause,
        SkipBack,
        SkipForward,
        VolumeUp,
        VolumeDown,
        ToggleMute,
        ToggleFullScreen,
        NextItem,
        PreviousItem,
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct PlayerAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = PlayerAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = PlayerAssets::iter()
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
    let context = Some("Player");
    use rmac_ui::shortcuts;
    cx.bind_keys([
        KeyBinding::new(shortcuts::OPEN.keystroke, OpenFile, None),
        KeyBinding::new(shortcuts::CLOSE.keystroke, CloseWindow, context),
        KeyBinding::new("space", PlayPause, context),
        KeyBinding::new("left", SkipBack, context),
        KeyBinding::new("right", SkipForward, context),
        KeyBinding::new("up", VolumeUp, context),
        KeyBinding::new("down", VolumeDown, context),
        KeyBinding::new("cmd-up", VolumeUp, context),
        KeyBinding::new("cmd-down", VolumeDown, context),
        KeyBinding::new("m", ToggleMute, context),
        KeyBinding::new("cmd-ctrl-f", ToggleFullScreen, context),
        KeyBinding::new("cmd-f", ToggleFullScreen, context),
        KeyBinding::new("cmd-right", NextItem, context),
        KeyBinding::new("cmd-left", PreviousItem, context),
    ]);
}

/// Open one window for `paths` (audio gets the compact controller window,
/// video a window at its own size once the first frame is known).
pub(crate) fn open_window(paths: Vec<PathBuf>, cx: &mut App) {
    let list = Playlist::new(paths);
    let audio = list.current().and_then(playlist::kind) == Some(Kind::Audio);
    let (width, height) = if audio {
        metrics::AUDIO_WINDOW
    } else {
        metrics::DEFAULT_VIDEO_WINDOW
    };
    let mut options = rmac_ui::window_options(width, height, cx);
    options.app_id = Some(PLAYER.to_owned());
    let name = list
        .current()
        .map(playlist::display_name)
        .unwrap_or_default();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(rmac_ui::native_window_title(&name, "Media Player").into());
    }
    if audio {
        let fixed = size(px(width), px(height));
        options.window_min_size = Some(fixed);
        options.is_resizable = false;
    } else {
        options.window_min_size = Some(size(
            px(metrics::MIN_VIDEO_WINDOW.0),
            px(metrics::MIN_VIDEO_WINDOW.1),
        ));
    }
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::track_key_window(window, cx);
            // Per-file windows share no saved geometry, so the in-window menu
            // strip (ADR 0023) is given here rather than by
            // `observe_window_state`; without it Alt opened nothing.
            rmac_ui::register_menu_strip_window(window);
            PlayerView::new(list, window, cx)
        });
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-player: could not open a window: {error}");
    }
    cx.activate(true);
}

/// What to do once the open panel closes.
#[derive(Debug, PartialEq, Eq)]
enum PanelOutcome {
    Open(Vec<PathBuf>),
    Stay,
    Quit,
}

/// A cancelled panel leaves the app running, windowless, like QuickTime
/// Player (MROBOT-05): File ▸ Open… is still in the menu bar. Only a panel
/// that could not be shown at all quits an app with no windows.
fn panel_outcome(chosen: Result<Vec<PathBuf>, String>, has_windows: bool) -> PanelOutcome {
    match chosen {
        Ok(paths) if !paths.is_empty() => PanelOutcome::Open(paths),
        Ok(_) => PanelOutcome::Stay,
        Err(error) => {
            eprintln!("rmac-player: {error}");
            if has_windows {
                PanelOutcome::Stay
            } else {
                PanelOutcome::Quit
            }
        }
    }
}

/// File ▸ Open…: the portal's open panel, then one window for the choice.
pub(crate) fn choose_and_open(cx: &mut App) {
    cx.spawn(async move |cx| {
        let chosen = rmac_portal::choose_media_files()
            .await
            .map_err(|error| error.to_string());
        cx.update(|cx| match panel_outcome(chosen, !cx.windows().is_empty()) {
            PanelOutcome::Open(paths) => open_window(paths, cx),
            PanelOutcome::Stay => {}
            PanelOutcome::Quit => cx.quit(),
        });
    })
    .detach();
}

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|argument| !argument.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
        .collect();
    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            bind_keys(cx);
            cx.on_action(|_: &OpenFile, cx| choose_and_open(cx));
            rmac_ui::install_app_menu(PLAYER, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            if Playlist::new(paths.clone()).is_empty() {
                choose_and_open(cx);
            } else {
                open_window(paths, cx);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_the_launch_panel_keeps_the_app_running() {
        assert_eq!(panel_outcome(Ok(Vec::new()), false), PanelOutcome::Stay);
        assert_eq!(panel_outcome(Ok(Vec::new()), true), PanelOutcome::Stay);
    }

    #[test]
    fn a_choice_opens_and_a_failed_panel_quits_only_when_windowless() {
        let file = PathBuf::from("/tmp/clip.mp4");
        assert_eq!(
            panel_outcome(Ok(vec![file.clone()]), false),
            PanelOutcome::Open(vec![file])
        );
        assert_eq!(
            panel_outcome(Err("no portal".into()), false),
            PanelOutcome::Quit
        );
        assert_eq!(
            panel_outcome(Err("no portal".into()), true),
            PanelOutcome::Stay
        );
    }
}
