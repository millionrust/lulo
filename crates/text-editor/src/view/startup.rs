//! Text Editor launch argument, document-window, and application startup authority.

use super::*;

/// TextEdit's plain-text window: 90 Menlo 11 columns by 30 lines plus the
/// 32 pt title bar, 656 × 422 (measured, design-lab/apps.html).
const WINDOW_WIDTH: f32 = 656.0;
const WINDOW_HEIGHT: f32 = 422.0;
const MAX_STARTUP_DOCUMENTS: usize = 32;

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
    let options = rmac_ui::window_options_for_app(
        rmac_ui::app_id::TEXT_EDITOR,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        cx,
    );
    cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::observe_window_state(rmac_ui::app_id::TEXT_EDITOR, window, cx);
            EditorView::new_with_path(initial_path, window, cx)
        });
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
        let path = std::path::absolute(path).ok()?.into_os_string().into_string().ok()?;
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
    let request = match parse_startup_request(std::env::args_os().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    // One process per app, as on the Mac: a Text Editor that is already
    // running opens these documents as new windows under its own menus.
    if hand_off_windows(&request).is_some_and(|windows| {
        rmac_ui::hand_off_to_running_instance(rmac_ui::app_id::TEXT_EDITOR, &windows)
    }) {
        return;
    }
    rmac_ui::application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            rmac_ui::install_app_instance(
                rmac_ui::app_id::TEXT_EDITOR,
                |arguments, cx| {
                    match parse_startup_request(arguments.into_iter().map(OsString::from)) {
                        Ok(request) => open_requested_windows(request, cx),
                        Err(message) => eprintln!("Text Editor ignored a window request: {message}"),
                    }
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
        let request = StartupRequest {
            open_untitled: true,
            paths: vec![PathBuf::from("/home/user/one.txt"), PathBuf::from("two.txt")],
        };
        let windows = hand_off_windows(&request).unwrap();
        assert_eq!(windows[0], ["--new-document"]);
        assert_eq!(windows[1], ["--", "/home/user/one.txt"]);
        // Relative paths are resolved here, where they were typed.
        assert!(std::path::Path::new(&windows[2][1]).is_absolute());
        assert!(windows[2][1].ends_with("two.txt"));
        // What the running editor receives parses back to the same request.
        let reparsed = windows
            .iter()
            .map(|arguments| {
                parse_startup_request(arguments.iter().map(OsString::from)).unwrap()
            })
            .collect::<Vec<_>>();
        assert!(reparsed[0].open_untitled && reparsed[0].paths.is_empty());
        assert_eq!(reparsed[1].paths, [PathBuf::from("/home/user/one.txt")]);
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
