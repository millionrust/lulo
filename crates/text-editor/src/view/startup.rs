//! Text Editor launch argument, document-window, and application startup authority.

use super::*;
use gpui::QuitMode;
use std::cell::RefCell;

thread_local! {
    static OPEN_DOCUMENTS: RefCell<Vec<gpui::WeakEntity<EditorView>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn document_window_count() -> usize {
    OPEN_DOCUMENTS.with(|documents| {
        documents
            .borrow()
            .iter()
            .filter(|view| view.upgrade().is_some())
            .count()
    })
}

/// Every path-backed document window open in this process right now, for
/// Application ▸ Quit and Keep Windows (TXT-MENU-001). An Untitled window
/// has no path to reopen here — its unsaved text, if any, already comes
/// back through the recovery store on the next launch.
pub(super) fn open_document_paths(cx: &App) -> Vec<PathBuf> {
    OPEN_DOCUMENTS.with(|documents| {
        documents
            .borrow()
            .iter()
            .filter_map(|view| view.upgrade())
            .filter_map(|view| view.read(cx).path.clone())
            .collect()
    })
}

fn track_document(view: &Entity<EditorView>) {
    OPEN_DOCUMENTS.with(|documents| {
        let mut documents = documents.borrow_mut();
        documents.retain(|view| view.upgrade().is_some());
        documents.push(view.downgrade());
    });
}

/// TextEdit's plain-text window: 90 Menlo 11 columns by 30 lines plus the
/// 32 pt title bar, 656 × 422 (measured, design-lab/apps.html).
const MAX_STARTUP_DOCUMENTS: usize = 32;

/// A rich document's own fixed default, 586 × 488 (measured, UIA-07):
/// unlike plain text, TextEdit does not size a new rich window from
/// Settings ▸ New Document ▸ Window Size at all — that field names plain
/// text's own columns/lines, which rich text has neither of.
const RICH_WINDOW_SIZE: (f32, f32) = (586.0, 488.0);

fn document_window_size(rich: bool) -> (f32, f32) {
    if rich {
        return RICH_WINDOW_SIZE;
    }
    let settings = crate::settings::current();
    (
        f32::from(settings.width_chars) * 6.56 + 26.0,
        f32::from(settings.height_lines) * f32::from(settings.font_size) * 13.0 / 11.0 + 32.0,
    )
}

#[derive(Debug, Eq, PartialEq)]
struct StartupRequest {
    open_untitled: bool,
    paths: Vec<PathBuf>,
}

fn parse_startup_request(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<StartupRequest, &'static str> {
    let mut paths = Vec::new();
    let mut open_untitled = false;
    let mut options = true;
    for argument in arguments {
        if options && argument == "--" {
            options = false;
            continue;
        }
        if options && argument == "--new-document" {
            open_untitled = true;
            continue;
        }
        if options
            && argument
                .to_str()
                .is_some_and(|value| value.starts_with('-'))
        {
            return Err("unsupported Text Editor launch option");
        }
        let path = PathBuf::from(argument);
        if path.as_os_str().is_empty() {
            return Err("empty Text Editor launch path");
        }
        if paths.len() == MAX_STARTUP_DOCUMENTS {
            return Err("too many Text Editor startup documents");
        }
        paths.push(path);
    }
    if paths.is_empty() && !open_untitled {
        open_untitled = true;
    }
    Ok(StartupRequest {
        open_untitled,
        paths,
    })
}

pub(super) fn open_editor_window(cx: &mut App, initial_path: Option<PathBuf>) -> Result<(), ()> {
    open_editor_window_with_picker(cx, initial_path, false)
}

fn open_editor_window_with_picker(
    cx: &mut App,
    initial_path: Option<PathBuf>,
    show_picker: bool,
) -> Result<(), ()> {
    // A brand-new document's size follows Settings ▸ New Document's own
    // format default; an existing file's follows its own extension, not
    // whatever a new document would open as today (UIA-07).
    let rich = match &initial_path {
        Some(path) => is_rich_text_path(path),
        None => crate::settings::current().rich_text_default,
    };
    let (width, height) = document_window_size(rich);
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::TEXT_EDITOR, width, height, cx);
    cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::observe_window_state(rmac_ui::app_id::TEXT_EDITOR, window, cx);
            EditorView::new_with_path(initial_path, show_picker, window, cx)
        });
        track_document(&view);
        cx.new(|cx| Root::new(view, window, cx))
    })
    .map(|_| ())
    .map_err(|_| ())
}

fn open_recent_without_window(index: usize, cx: &mut App) {
    if document_window_count() != 0 {
        return;
    }
    cx.spawn(async move |cx| {
        let path = cx
            .background_executor()
            .spawn(async move {
                let store = rmac_recent_documents::Store::from_environment().ok()?;
                let mut paths = store.load_for_app(rmac_ui::app_id::TEXT_EDITOR).ok()?;
                (index < paths.len()).then(|| paths.swap_remove(index))
            })
            .await;
        if let Some(path) = path {
            cx.update(|cx| {
                let _ = open_editor_window(cx, Some(path));
            });
        }
    })
    .detach();
}

fn clear_recent_without_window(cx: &mut App) {
    if document_window_count() != 0 {
        return;
    }
    cx.background_executor()
        .spawn(async move {
            if let Ok(store) = rmac_recent_documents::Store::from_environment() {
                let _ = store.clear_for_app(rmac_ui::app_id::TEXT_EDITOR);
            }
        })
        .detach();
}

/// TextEdit's File ▸ Duplicate: a new untitled window seeded with `content`,
/// already marked unsaved (a duplicate has never been written to disk). The
/// new window has no path, so its own Save behaves like Save As.
pub(super) fn open_duplicate_window(
    cx: &mut App,
    content: super::DuplicateContent,
) -> Result<(), ()> {
    // A duplicate keeps its source's own format (TE's File ▸ Duplicate
    // copies formatting exactly), so it sizes like that format, not
    // whatever Settings ▸ New Document's default happens to be right now.
    let (width, height) = document_window_size(content.rich.is_some());
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::TEXT_EDITOR, width, height, cx);
    cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::observe_window_state(rmac_ui::app_id::TEXT_EDITOR, window, cx);
            let mut view = EditorView::new_with_path(None, false, window, cx);
            view.seed_duplicate_content(content, window, cx);
            view
        });
        track_document(&view);
        cx.new(|cx| Root::new(view, window, cx))
    })
    .map(|_| ())
    .map_err(|_| ())
}

/// The windows a launch asks for, as the running process's `OpenWindow`
/// arguments: one list per window, paths made absolute because the running
/// process has its own working directory. `None` when a path cannot travel
/// over D-Bus (it is not UTF-8); this launch then opens it itself.
fn hand_off_windows(request: &StartupRequest) -> Option<Vec<Vec<String>>> {
    let mut windows = Vec::new();
    if request.open_untitled {
        windows.push(vec!["--new-document".to_owned()]);
    }
    for path in &request.paths {
        let path = std::path::absolute(path)
            .ok()?
            .into_os_string()
            .into_string()
            .ok()?;
        windows.push(vec!["--".to_owned(), path]);
    }
    Some(windows)
}

fn open_requested_windows(request: StartupRequest, cx: &mut App) {
    if request.open_untitled && open_editor_window(cx, None).is_err() {
        eprintln!("Text Editor could not open a document window");
    }
    for path in request.paths {
        if open_editor_window(cx, Some(path)).is_err() {
            eprintln!("Text Editor could not open a document window");
        }
    }
}

pub(crate) fn run() {
    crate::settings::initialize();
    let mut request = match parse_startup_request(std::env::args_os().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    // Application ▸ Quit and Keep Windows (TXT-MENU-001): reopen whatever
    // this process had open last time, on top of whatever this launch's
    // own arguments ask for. A bare launch that only exists to restore a
    // kept session should not also open a blank Untitled window.
    let kept = recovery_state::take_kept_session();
    if !kept.is_empty() && request.paths.is_empty() {
        request.open_untitled = false;
    }
    for path in kept {
        if request.paths.len() >= MAX_STARTUP_DOCUMENTS {
            break;
        }
        if !request.paths.contains(&path) {
            request.paths.push(path);
        }
    }
    // One process per app, as on the Mac: a Text Editor that is already
    // running opens these documents as new windows under its own menus.
    if hand_off_windows(&request).is_some_and(|windows| {
        rmac_ui::hand_off_to_running_instance(rmac_ui::app_id::TEXT_EDITOR, &windows)
    }) {
        return;
    }
    rmac_ui::application()
        .with_assets(gpui_component_assets::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.on_action(|_: &NewFile, cx| {
                if document_window_count() == 0 {
                    let _ = open_editor_window(cx, None);
                }
            });
            cx.on_action(|_: &OpenFile, cx| {
                if document_window_count() == 0 {
                    let _ = open_editor_window_with_picker(cx, None, true);
                }
            });
            cx.on_action(|_: &ShowSettings, cx| {
                if document_window_count() == 0 {
                    crate::settings_window::show(cx);
                }
            });
            cx.on_action(|_: &crate::OpenRecent0, cx| open_recent_without_window(0, cx));
            cx.on_action(|_: &crate::OpenRecent1, cx| open_recent_without_window(1, cx));
            cx.on_action(|_: &crate::OpenRecent2, cx| open_recent_without_window(2, cx));
            cx.on_action(|_: &crate::OpenRecent3, cx| open_recent_without_window(3, cx));
            cx.on_action(|_: &crate::OpenRecent4, cx| open_recent_without_window(4, cx));
            cx.on_action(|_: &crate::OpenRecent5, cx| open_recent_without_window(5, cx));
            cx.on_action(|_: &crate::OpenRecent6, cx| open_recent_without_window(6, cx));
            cx.on_action(|_: &crate::OpenRecent7, cx| open_recent_without_window(7, cx));
            cx.on_action(|_: &crate::OpenRecent8, cx| open_recent_without_window(8, cx));
            cx.on_action(|_: &crate::OpenRecent9, cx| open_recent_without_window(9, cx));
            cx.on_action(|_: &crate::ClearRecentMenu, cx| clear_recent_without_window(cx));
            rmac_ui::install_app_instance(
                rmac_ui::app_id::TEXT_EDITOR,
                |arguments, cx| match parse_startup_request(
                    arguments.into_iter().map(OsString::from),
                ) {
                    Ok(request) => open_requested_windows(request, cx),
                    Err(message) => eprintln!("Text Editor ignored a window request: {message}"),
                },
                cx,
            );
            open_requested_windows(request, cx);
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn desktop_launch_paths_open_independent_windows() {
        assert_eq!(
            parse_startup_request([
                OsString::from("/home/user/one.txt"),
                OsString::from("/home/user/two.rtf"),
            ])
            .unwrap(),
            StartupRequest {
                open_untitled: false,
                paths: vec![
                    PathBuf::from("/home/user/one.txt"),
                    PathBuf::from("/home/user/two.rtf"),
                ],
            }
        );
        assert_eq!(
            parse_startup_request([OsString::from("--new-document")]).unwrap(),
            StartupRequest {
                open_untitled: true,
                paths: Vec::new(),
            }
        );
    }

    #[test]
    fn a_second_launch_hands_its_documents_to_the_running_editor() {
        // An absolute path on every platform (Windows needs a drive).
        let one = if cfg!(windows) {
            r"C:\home\user\one.txt"
        } else {
            "/home/user/one.txt"
        };
        let request = StartupRequest {
            open_untitled: true,
            paths: vec![PathBuf::from(one), PathBuf::from("two.txt")],
        };
        let windows = hand_off_windows(&request).unwrap();
        assert_eq!(windows[0], ["--new-document"]);
        assert_eq!(windows[1], ["--", one]);
        // Relative paths are resolved here, where they were typed.
        assert!(std::path::Path::new(&windows[2][1]).is_absolute());
        assert!(windows[2][1].ends_with("two.txt"));
        // What the running editor receives parses back to the same request.
        let reparsed = windows
            .iter()
            .map(|arguments| parse_startup_request(arguments.iter().map(OsString::from)).unwrap())
            .collect::<Vec<_>>();
        assert!(reparsed[0].open_untitled && reparsed[0].paths.is_empty());
        assert_eq!(reparsed[1].paths, [PathBuf::from(one)]);
        assert!(!reparsed[1].open_untitled);
    }

    #[test]
    fn startup_arguments_are_bounded_and_options_fail_closed() {
        assert!(parse_startup_request([OsString::from("--unknown")]).is_err());
        assert_eq!(
            parse_startup_request([OsString::from("--"), OsString::from("-literal-name.txt"),])
                .unwrap()
                .paths,
            vec![PathBuf::from("-literal-name.txt")]
        );
        assert!(parse_startup_request(
            (0..=MAX_STARTUP_DOCUMENTS).map(|index| OsString::from(format!("/tmp/{index}")))
        )
        .is_err());
    }
}
