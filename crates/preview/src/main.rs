//! rmac Preview: macOS Preview for images and PDF documents.

mod view;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::{App, AppContext as _, AssetSource, KeyBinding, Result, SharedString};
use gpui_component::Root;
use rmac_preview::document::{self, Kind};
use rmac_preview::metrics;
use rmac_preview::render;
use rmac_ui::app_id::PREVIEW;

use crate::view::PreviewView;

gpui::actions!(
    preview,
    [
        OpenFile,
        CloseWindow,
        Copy,
        Find,
        FindNext,
        FindPrevious,
        HideSidebar,
        ShowThumbnails,
        ActualSize,
        ZoomToFit,
        ZoomIn,
        ZoomOut,
        PreviousItem,
        NextItem,
        ShowInspector,
        RotateLeft,
        RotateRight,
        SelectAll,
        GoToPage,
        PrintDocument,
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct PreviewAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = PreviewAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = PreviewAssets::iter()
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
    let context = Some("Preview");
    use rmac_ui::shortcuts;
    cx.bind_keys([
        KeyBinding::new(shortcuts::OPEN.keystroke, OpenFile, None),
        KeyBinding::new(shortcuts::CLOSE.keystroke, CloseWindow, context),
        KeyBinding::new(shortcuts::COPY.keystroke, Copy, context),
        KeyBinding::new(shortcuts::SELECT_ALL.keystroke, SelectAll, context),
        KeyBinding::new("alt-cmd-g", GoToPage, context),
        KeyBinding::new(shortcuts::PRINT.keystroke, PrintDocument, context),
        KeyBinding::new(shortcuts::FIND.keystroke, Find, context),
        KeyBinding::new(shortcuts::FIND_NEXT.keystroke, FindNext, context),
        KeyBinding::new(shortcuts::FIND_PREVIOUS.keystroke, FindPrevious, context),
        KeyBinding::new("alt-cmd-1", HideSidebar, context),
        KeyBinding::new("alt-cmd-2", ShowThumbnails, context),
        KeyBinding::new(shortcuts::ZOOM_RESET.keystroke, ActualSize, context),
        KeyBinding::new("cmd-9", ZoomToFit, context),
        KeyBinding::new(shortcuts::ZOOM_IN.keystroke, ZoomIn, context),
        KeyBinding::new(shortcuts::ZOOM_IN_ALTERNATE.keystroke, ZoomIn, context),
        KeyBinding::new(shortcuts::ZOOM_OUT.keystroke, ZoomOut, context),
        KeyBinding::new("alt-up", PreviousItem, context),
        KeyBinding::new("alt-down", NextItem, context),
        KeyBinding::new(shortcuts::INFO.keystroke, ShowInspector, context),
        KeyBinding::new("cmd-l", RotateLeft, context),
        KeyBinding::new("cmd-r", RotateRight, context),
    ]);
}

/// Initial window size: an image opens at its own size under the toolbar,
/// anything else at Preview's measured default.
fn initial_size(paths: &[PathBuf]) -> (f32, f32) {
    let Some(first) = paths.first() else {
        return metrics::DEFAULT_WINDOW;
    };
    match render::sniff_path(first) {
        Ok(Kind::Image(_)) if paths.len() == 1 => render::image_dimensions(first)
            .map(|(width, height)| metrics::image_window_size((width as f32, height as f32)))
            .unwrap_or(metrics::DEFAULT_WINDOW),
        _ => metrics::DEFAULT_WINDOW,
    }
}

pub(crate) fn open_window(paths: Vec<PathBuf>, cx: &mut App) {
    let (width, height) = initial_size(&paths);
    let mut options = rmac_ui::window_options(width, height, cx);
    options.app_id = Some(PREVIEW.to_owned());
    let name = paths
        .first()
        .map(|path| document::display_name(path))
        .unwrap_or_default();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(rmac_ui::native_window_title(&name, "Preview").into());
    }
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| PreviewView::new(paths, window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-preview: could not open a window: {error}");
    }
    cx.activate(true);
}

/// File ▸ Open…: the portal's open panel, then one window for the choice.
pub(crate) fn choose_and_open(quit_if_cancelled: bool, cx: &mut App) {
    cx.spawn(async move |cx| {
        let chosen = rmac_portal::choose_preview_documents().await;
        cx.update(|cx| match chosen {
            Ok(paths) if !paths.is_empty() => open_window(paths, cx),
            Ok(_) if quit_if_cancelled && cx.windows().is_empty() => cx.quit(),
            Ok(_) => {}
            Err(error) => {
                eprintln!("rmac-preview: {error}");
                if cx.windows().is_empty() {
                    // Nothing to show and no way to choose: open the empty
                    // window so the failure is visible instead of silent.
                    open_window(Vec::new(), cx);
                }
            }
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
            cx.on_action(|_: &OpenFile, cx| choose_and_open(false, cx));
            rmac_ui::install_app_menu(PREVIEW, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            if paths.is_empty() {
                choose_and_open(true, cx);
            } else {
                open_window(paths, cx);
            }
        });
}
