//! rmac Preview: macOS Preview for images and PDF documents.

mod view;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::{
    App, AppContext as _, AssetSource, KeyBinding, QuitMode, Result, SharedString, WeakEntity,
};
#[cfg(not(target_os = "linux"))]
use gpui::{ClipboardEntry, ImageFormat};
use gpui_component::Root;
use rmac_preview::document::{self, Kind};
use rmac_preview::markup;
use rmac_preview::metrics;
use rmac_preview::render;
use rmac_ui::app_id::PREVIEW;

use crate::view::PreviewView;

gpui::actions!(
    preview,
    [
        OpenFile,
        NewFromClipboard,
        QuitAndKeepWindows,
        CloseWindow,
        CloseAll,
        CloseSelected,
        Copy,
        DeleteSelection,
        MoveToTrash,
        Find,
        FindNext,
        FindPrevious,
        UseSelectionForFind,
        JumpToSelection,
        HideSidebar,
        ShowThumbnails,
        ShowBookmarks,
        AddBookmark,
        ShowImageBackground,
        ActualSize,
        ZoomToFit,
        ZoomIn,
        ZoomOut,
        ActualSizeOnAll,
        ZoomAllToFit,
        ZoomAllIn,
        ZoomAllOut,
        PreviousItem,
        NextItem,
        PreviousDocument,
        NextDocument,
        PageUp,
        PageDown,
        ShowInspector,
        RotateLeft,
        RotateRight,
        AnnotateHighlight,
        AnnotateUnderline,
        AnnotateStrikeThrough,
        AnnotateRectangle,
        AnnotateSignature,
        AnnotateArrow,
        AnnotateOval,
        AnnotateLine,
        AnnotateText,
        SelectAll,
        GoToPage,
        Back,
        Forward,
        PrintDocument,
        ExportAsPdf,
        SaveAs,
        ToggleToolbar,
        ToggleMarkup,
        EnterFullScreen,
        SaveMarkup,
        RevertMarkup,
        UndoMarkup,
        RedoMarkup,
        // File ▸ Open Recent ▸ (PREV-08/PREV-15): one action per shown row,
        // up to `rmac_app_menu::recent::MAX_ENTRIES`, plus "Clear Menu".
        OpenRecent0,
        OpenRecent1,
        OpenRecent2,
        OpenRecent3,
        OpenRecent4,
        OpenRecent5,
        OpenRecent6,
        OpenRecent7,
        OpenRecent8,
        OpenRecent9,
        ClearRecentMenu,
    ]
);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct PreviewAssets;

struct CombinedAssets;

thread_local! {
    /// Weak references keep closed document windows out of a saved session.
    static OPEN_VIEWS: std::cell::RefCell<Vec<WeakEntity<PreviewView>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

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
        KeyBinding::new("cmd-n", NewFromClipboard, None),
        KeyBinding::new("alt-cmd-q", QuitAndKeepWindows, None),
        KeyBinding::new(shortcuts::CLOSE.keystroke, CloseWindow, context),
        KeyBinding::new("alt-cmd-w", CloseAll, None),
        KeyBinding::new("shift-cmd-w", CloseSelected, None),
        KeyBinding::new(shortcuts::COPY.keystroke, Copy, context),
        KeyBinding::new("cmd-backspace", MoveToTrash, context),
        KeyBinding::new(shortcuts::SELECT_ALL.keystroke, SelectAll, context),
        KeyBinding::new("alt-cmd-g", GoToPage, context),
        KeyBinding::new("cmd-[", Back, context),
        KeyBinding::new("cmd-]", Forward, context),
        KeyBinding::new(shortcuts::PRINT.keystroke, PrintDocument, context),
        KeyBinding::new("shift-cmd-a", ToggleMarkup, context),
        KeyBinding::new("cmd-s", SaveMarkup, context),
        KeyBinding::new("alt-shift-cmd-s", SaveAs, context),
        KeyBinding::new("alt-cmd-t", ToggleToolbar, context),
        KeyBinding::new("cmd-z", UndoMarkup, context),
        KeyBinding::new("shift-cmd-z", RedoMarkup, context),
        KeyBinding::new(shortcuts::FIND.keystroke, Find, context),
        KeyBinding::new(shortcuts::FIND_NEXT.keystroke, FindNext, context),
        KeyBinding::new(shortcuts::FIND_PREVIOUS.keystroke, FindPrevious, context),
        KeyBinding::new("cmd-e", UseSelectionForFind, context),
        KeyBinding::new("cmd-j", JumpToSelection, context),
        KeyBinding::new("alt-cmd-1", HideSidebar, context),
        KeyBinding::new("alt-cmd-2", ShowThumbnails, context),
        KeyBinding::new("alt-cmd-5", ShowBookmarks, context),
        KeyBinding::new("cmd-d", AddBookmark, context),
        KeyBinding::new("alt-cmd-b", ShowImageBackground, context),
        KeyBinding::new(shortcuts::ZOOM_RESET.keystroke, ActualSize, context),
        KeyBinding::new("cmd-9", ZoomToFit, context),
        KeyBinding::new("alt-cmd-0", ActualSizeOnAll, context),
        KeyBinding::new("alt-cmd-9", ZoomAllToFit, context),
        KeyBinding::new("alt-cmd-+", ZoomAllIn, context),
        KeyBinding::new("alt-cmd--", ZoomAllOut, context),
        KeyBinding::new(shortcuts::ZOOM_IN.keystroke, ZoomIn, context),
        KeyBinding::new(shortcuts::ZOOM_IN_ALTERNATE.keystroke, ZoomIn, context),
        KeyBinding::new(shortcuts::ZOOM_OUT.keystroke, ZoomOut, context),
        KeyBinding::new("alt-up", PreviousItem, context),
        KeyBinding::new("alt-down", NextItem, context),
        KeyBinding::new("alt-pageup", PreviousDocument, context),
        KeyBinding::new("alt-pagedown", NextDocument, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new(shortcuts::INFO.keystroke, ShowInspector, context),
        KeyBinding::new("cmd-l", RotateLeft, context),
        KeyBinding::new("cmd-r", RotateRight, context),
        KeyBinding::new("ctrl-cmd-h", AnnotateHighlight, context),
        KeyBinding::new("ctrl-cmd-u", AnnotateUnderline, context),
        KeyBinding::new("ctrl-cmd-s", AnnotateStrikeThrough, context),
        KeyBinding::new("ctrl-cmd-r", AnnotateRectangle, context),
        KeyBinding::new("ctrl-cmd-a", AnnotateArrow, context),
        KeyBinding::new("ctrl-cmd-o", AnnotateOval, context),
        KeyBinding::new("ctrl-cmd-i", AnnotateLine, context),
        KeyBinding::new("ctrl-cmd-t", AnnotateText, context),
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

#[cfg(not(target_os = "linux"))]
fn clipboard_image(cx: &App) -> Option<(Vec<u8>, &'static str)> {
    cx.read_from_clipboard()?.into_entries().find_map(|entry| {
        let ClipboardEntry::Image(image) = entry else {
            return None;
        };
        let extension = match image.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            _ => return None,
        };
        Some((image.bytes, extension))
    })
}

#[cfg(target_os = "linux")]
fn preferred_clipboard_image_type(offered: &str) -> Option<(&'static str, &'static str)> {
    [
        ("image/png", "png"),
        ("image/jpeg", "jpg"),
        ("image/webp", "webp"),
        ("image/gif", "gif"),
        ("image/bmp", "bmp"),
        ("image/tiff", "tiff"),
    ]
    .into_iter()
    .find(|(mime, _)| offered.lines().any(|line| line.trim() == *mime))
}

#[cfg(target_os = "linux")]
fn clipboard_image_type() -> Option<(&'static str, &'static str)> {
    let output = std::process::Command::new("wl-paste")
        .arg("--list-types")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    preferred_clipboard_image_type(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "linux")]
pub(crate) fn clipboard_image_available() -> bool {
    clipboard_image_type().is_some()
}

#[cfg(target_os = "linux")]
fn clipboard_image() -> Option<(Vec<u8>, &'static str)> {
    use std::io::Read as _;

    let (mime, extension) = clipboard_image_type()?;
    let mut child = std::process::Command::new("wl-paste")
        .args(["--type", mime])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    const MAX_CLIPBOARD_IMAGE: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()?
        .take(MAX_CLIPBOARD_IMAGE + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > MAX_CLIPBOARD_IMAGE {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    child.wait().ok()?.success().then_some((bytes, extension))
}

/// A clipboard image is an independent document. Keep its backing bytes in
/// the app cache until Save As gives it a permanent location.
fn open_clipboard_image(bytes: Vec<u8>, extension: &'static str, cx: &mut App) {
    static NEXT_CLIPBOARD_DOCUMENT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let id = NEXT_CLIPBOARD_DOCUMENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = cache
        .join("rmac-preview/clipboard")
        .join(format!("{}-{id}/Untitled.{extension}", std::process::id()));
    cx.spawn(async move |cx| {
        let result = blocking::unblock(move || -> std::io::Result<PathBuf> {
            std::fs::create_dir_all(path.parent().expect("clipboard path has a parent"))?;
            std::fs::write(&path, bytes)?;
            Ok(path)
        })
        .await;
        match result {
            Ok(path) => cx.update(|cx| open_window(vec![path], cx)),
            Err(error) => eprintln!("rmac-preview: could not open clipboard image: {error}"),
        }
    })
    .detach();
}

fn new_from_clipboard(cx: &mut App) {
    #[cfg(target_os = "linux")]
    cx.spawn(async move |cx| {
        if let Some((bytes, extension)) = blocking::unblock(clipboard_image).await {
            cx.update(|cx| open_clipboard_image(bytes, extension, cx));
        }
    })
    .detach();

    #[cfg(not(target_os = "linux"))]
    if let Some((bytes, extension)) = clipboard_image(cx) {
        open_clipboard_image(bytes, extension, cx);
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
        let view = cx.new(|cx| {
            rmac_ui::track_key_window(window, cx);
            PreviewView::new(paths, window, cx)
        });
        let focus = view.read(cx).focus.clone();
        OPEN_VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            views.retain(|weak| weak.upgrade().is_some());
            views.push(view.downgrade());
        });
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-preview: could not open a window: {error}");
    }
    cx.activate(true);
}

fn saved_windows_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir);
    state.join("rmac-preview/saved-windows.json")
}

/// App ▸ Quit and Keep Windows: persist just the documents in each live
/// window, then quit after the write finishes. The state lives in XDG state
/// so the next launch can reopen the same window groups once.
fn quit_and_keep_windows(cx: &mut App) {
    let (windows, jobs) = OPEN_VIEWS.with(|views| {
        let open = views.borrow();
        let mut windows = Vec::new();
        let mut jobs = Vec::new();
        for view in open.iter().filter_map(WeakEntity::upgrade) {
            let view = view.read(cx);
            let paths = view.open_paths();
            if !paths.is_empty() {
                windows.push(paths);
                jobs.extend(view.pending_markup());
            }
        }
        (windows, jobs)
    });
    cx.spawn(async move |cx| {
        let result = blocking::unblock(move || -> std::io::Result<()> {
            for (source, original, items) in jobs {
                let base = original.unwrap_or_else(|| source.clone());
                let temporary =
                    source.with_extension(format!("lulo-quitting-{}.pdf", std::process::id()));
                markup::write_pdf(&base, &temporary, &items).map_err(std::io::Error::other)?;
                std::fs::rename(&temporary, &source)?;
            }
            let path = saved_windows_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let bytes = serde_json::to_vec(&windows).map_err(std::io::Error::other)?;
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(temporary, path)
        })
        .await;
        if let Err(error) = result {
            eprintln!("rmac-preview: could not keep windows: {error}");
            return;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Close every Preview window after saving any dirty PDF annotations. Keep
/// the writes off the UI thread, then close the windows that existed when
/// the command was invoked.
fn close_all(cx: &mut App) {
    let windows = cx.windows();
    let jobs = OPEN_VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .filter_map(WeakEntity::upgrade)
            .flat_map(|view| view.read(cx).pending_markup())
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let result = blocking::unblock(move || -> std::io::Result<()> {
            for (source, original, items) in jobs {
                let base = original.unwrap_or_else(|| source.clone());
                let temporary =
                    source.with_extension(format!("lulo-closing-{}.pdf", std::process::id()));
                markup::write_pdf(&base, &temporary, &items).map_err(std::io::Error::other)?;
                std::fs::rename(temporary, source)?;
            }
            Ok(())
        })
        .await;
        match result {
            Ok(()) => cx.update(|cx| {
                for handle in windows {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }),
            Err(error) => eprintln!("rmac-preview: save on close all failed: {error}"),
        }
    })
    .detach();
}

/// Consume the saved session once. Reading and removing the file stays off
/// the UI thread, including when a home directory is slow.
fn restore_kept_windows(initial_paths: Vec<PathBuf>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let restored = blocking::unblock(move || {
            let path = saved_windows_path();
            let bytes = std::fs::read(&path).ok()?;
            let _ = std::fs::remove_file(path);
            serde_json::from_slice::<Vec<Vec<PathBuf>>>(&bytes).ok()
        })
        .await
        .unwrap_or_default();
        cx.update(|cx| {
            for paths in restored {
                if !paths.is_empty() {
                    open_window(paths, cx);
                }
            }
            if initial_paths.is_empty() {
                if cx.windows().is_empty() {
                    choose_and_open(true, cx);
                }
            } else {
                open_window(initial_paths, cx);
            }
        });
    })
    .detach();
}

/// File ▸ Open Recent ▸ (PREV-08/PREV-15): opens the document at `index` in
/// the store's own File-Open-Recent list, re-read now (off the render
/// thread) rather than cached from when the menu opened, in a new window —
/// exactly what File ▸ Open… does with a chosen path. A document the store
/// no longer lists (moved, deleted, or the list simply changed since the
/// menu opened) is silently skipped.
fn open_recent_menu_entry(index: usize, cx: &mut App) {
    cx.spawn(async move |cx| {
        let path = cx
            .background_executor()
            .spawn(async move {
                let store = rmac_recent_documents::Store::from_environment().ok()?;
                let mut paths = store.load_for_app(PREVIEW).ok()?;
                (index < paths.len()).then(|| paths.swap_remove(index))
            })
            .await;
        if let Some(path) = path {
            cx.update(|cx| open_window(vec![path], cx));
        }
    })
    .detach();
}

/// File ▸ Open Recent ▸ Clear Menu (PREV-08/PREV-15): removes only what
/// Preview itself recorded, off the render thread. The menu bar sees the
/// change next time it opens the menu, since it is built fresh from the
/// store then; nothing needs to be published.
fn clear_recent_documents(cx: &mut App) {
    cx.background_executor()
        .spawn(async move {
            if let Ok(store) = rmac_recent_documents::Store::from_environment() {
                let _ = store.clear_for_app(PREVIEW);
            }
        })
        .detach();
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

/// A running Preview's `OpenWindow` requests for these documents: one
/// window for all of them, as a launch opens, split only where a request's
/// argument limit forces it. An empty request asks for the Open panel.
/// `None` when a path cannot travel over D-Bus (it is not UTF-8).
fn hand_off_windows(paths: &[PathBuf]) -> Option<Vec<Vec<String>>> {
    const PATHS_PER_REQUEST: usize = 8;
    if paths.is_empty() {
        return Some(vec![Vec::new()]);
    }
    let absolute = paths
        .iter()
        .map(|path| {
            std::path::absolute(path)
                .ok()?
                .into_os_string()
                .into_string()
                .ok()
        })
        .collect::<Option<Vec<_>>>()?;
    Some(
        absolute
            .chunks(PATHS_PER_REQUEST)
            .map(<[String]>::to_vec)
            .collect(),
    )
}

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|argument| !argument.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
        .collect();
    // One process per app, as on the Mac: a running Preview opens these
    // documents in a new window under its own menus.
    if hand_off_windows(&paths)
        .is_some_and(|windows| rmac_ui::hand_off_to_running_instance(PREVIEW, &windows))
    {
        return;
    }
    rmac_ui::application()
        .with_assets(CombinedAssets)
        // GPUI's own default on Linux (`QuitMode::Default`) quits the whole
        // process the instant the last window closes — the actual cause of
        // ⌘W exiting Preview instead of leaving it running with no windows
        // open, like Finder (behavior:preview/close-window). Several
        // shell binaries (rmac-dock, rmac-menubar, …) already opt out of
        // this for the same reason.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            bind_keys(cx);
            rmac_ui::set_menu_enabled("preview::NewFromClipboard", false, cx);
            cx.on_window_closed(|cx, _| {
                let has_document_window = OPEN_VIEWS
                    .with(|views| views.borrow().iter().any(|view| view.upgrade().is_some()));
                if !has_document_window {
                    view::disable_document_menu(cx);
                }
            })
            .detach();
            cx.on_action(|_: &QuitAndKeepWindows, cx| quit_and_keep_windows(cx));
            cx.on_action(|_: &OpenFile, cx| choose_and_open(false, cx));
            cx.on_action(|_: &NewFromClipboard, cx| new_from_clipboard(cx));
            cx.on_action(|_: &OpenRecent0, cx| open_recent_menu_entry(0, cx));
            cx.on_action(|_: &OpenRecent1, cx| open_recent_menu_entry(1, cx));
            cx.on_action(|_: &OpenRecent2, cx| open_recent_menu_entry(2, cx));
            cx.on_action(|_: &OpenRecent3, cx| open_recent_menu_entry(3, cx));
            cx.on_action(|_: &OpenRecent4, cx| open_recent_menu_entry(4, cx));
            cx.on_action(|_: &OpenRecent5, cx| open_recent_menu_entry(5, cx));
            cx.on_action(|_: &OpenRecent6, cx| open_recent_menu_entry(6, cx));
            cx.on_action(|_: &OpenRecent7, cx| open_recent_menu_entry(7, cx));
            cx.on_action(|_: &OpenRecent8, cx| open_recent_menu_entry(8, cx));
            cx.on_action(|_: &OpenRecent9, cx| open_recent_menu_entry(9, cx));
            cx.on_action(|_: &ClearRecentMenu, cx| clear_recent_documents(cx));
            rmac_ui::install_app_instance(
                PREVIEW,
                |arguments, cx| {
                    let paths = arguments
                        .into_iter()
                        .filter(|argument| !argument.starts_with("--"))
                        .map(PathBuf::from)
                        .collect::<Vec<_>>();
                    if paths.is_empty() {
                        choose_and_open(false, cx);
                    } else {
                        open_window(paths, cx);
                    }
                },
                cx,
            );
            // Preview stays running with no windows open, like Finder
            // (behavior:preview/close-window, behavior:files/close-window-cmd-w):
            // ⌘W on the last document window used to quit the whole
            // process instead of just closing it, and quitting would also
            // undercut the hand-off above, which needs the process alive
            // to open a later document in a new window rather than
            // relaunching.
            restore_kept_windows(paths, cx);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn clipboard_image_type_prefers_png_and_ignores_text() {
        assert_eq!(preferred_clipboard_image_type("text/plain\n"), None);
        assert_eq!(
            preferred_clipboard_image_type("text/plain\nimage/jpeg\nimage/png\n"),
            Some(("image/png", "png"))
        );
    }

    #[test]
    fn a_second_launch_hands_its_documents_to_the_running_preview() {
        assert_eq!(hand_off_windows(&[]), Some(vec![Vec::new()]));
        let paths = (0..10)
            .map(|index| PathBuf::from(format!("/home/user/scan-{index}.png")))
            .collect::<Vec<_>>();
        let windows = hand_off_windows(&paths).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].len(), 8);
        assert_eq!(
            windows[1],
            ["/home/user/scan-8.png", "/home/user/scan-9.png"]
        );
        let relative = hand_off_windows(&[PathBuf::from("photo.jpg")]).unwrap();
        assert!(std::path::Path::new(&relative[0][0]).is_absolute());
    }
}
