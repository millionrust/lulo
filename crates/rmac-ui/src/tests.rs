use gpui::{SharedString, WindowBounds, WindowDecorations, WindowOptions};

use super::{
    app_id, native_window_title,
    window::{outer_window_size, window_options_for_app_with_bounds},
};

fn native_title(options: &WindowOptions) -> Option<&SharedString> {
    options
        .titlebar
        .as_ref()
        .and_then(|titlebar| titlebar.title.as_ref())
}

#[test]
fn window_options_publish_the_exact_application_id() {
    assert_eq!(
        window_options_for_app_with_bounds(
            app_id::TEXT_EDITOR,
            800.0,
            600.0,
            WindowBounds::default(),
        )
        .app_id,
        Some(app_id::TEXT_EDITOR.to_owned())
    );
    assert_eq!(
        window_options_for_app_with_bounds(app_id::FILES, 800.0, 600.0, WindowBounds::default(),)
            .app_id,
        Some(app_id::FILES.to_owned())
    );
    assert_eq!(
        window_options_for_app_with_bounds(app_id::NOTES, 1080.0, 720.0, WindowBounds::default(),)
            .app_id,
        Some(app_id::NOTES.to_owned())
    );
}

#[test]
fn app_windows_request_only_rmac_client_decorations() {
    let options =
        window_options_for_app_with_bounds(app_id::NOTES, 1080.0, 720.0, WindowBounds::default());

    assert_eq!(options.window_decorations, Some(WindowDecorations::Client));
}

#[test]
fn identified_window_options_publish_stable_native_titles() {
    let cases = [
        (app_id::FILES, "Files"),
        (app_id::TERMINAL, "Terminal"),
        (app_id::NOTES, "Notes"),
        (app_id::TEXT_EDITOR, "Text Editor"),
        (app_id::SYSTEM_MONITOR, "System Monitor"),
        (app_id::APP_DRAWER, "Apps"),
        (app_id::SYSTEM_SETTINGS, "Settings"),
        (app_id::CALCULATOR, "Calculator"),
        (app_id::PREVIEW, "Preview"),
        (app_id::CLOCK, "Clock"),
        (app_id::WEATHER, "Weather"),
        (app_id::PLAYER, "Media Player"),
    ];

    for (app_id, expected) in cases {
        let options =
            window_options_for_app_with_bounds(app_id, 800.0, 600.0, WindowBounds::default());
        assert_eq!(
            native_title(&options).map(|title| title.as_ref()),
            Some(expected)
        );
    }
}

#[test]
fn live_native_titles_are_bounded_and_spoof_resistant() {
    assert_eq!(
        native_window_title("  report.txt\n\u{202e}gpj.exe  ", "Text Editor"),
        "report.txt gpj.exe — Text Editor"
    );
    assert_eq!(native_window_title("Terminal", "Terminal"), "Terminal");
    assert_eq!(native_window_title("\n\t", "Finder"), "Finder");

    let title = native_window_title(&format!("{}😀", "a".repeat(300)), "Terminal");
    assert!(title.len() <= 256);
    assert!(title.ends_with(" — Terminal"));
    assert!(title.is_char_boundary(title.len()));
}

#[test]
fn app_windows_open_at_the_visible_size_plus_the_client_frame() {
    let (width, height) = outer_window_size(580.0, 385.0);
    if cfg!(target_os = "linux") {
        assert_eq!((width, height), (604.0, 409.0));
    } else {
        assert_eq!((width, height), (580.0, 385.0));
    }
}
