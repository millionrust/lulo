use gpui::{SharedString, WindowBounds, WindowOptions};

use super::{app_id, native_window_title, window::window_options_for_app_with_bounds};

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
fn identified_window_options_publish_stable_native_titles() {
    let cases = [
        (app_id::FILES, "Finder"),
        (app_id::TERMINAL, "Terminal"),
        (app_id::NOTES, "Notes"),
        (app_id::TEXT_EDITOR, "Text Editor"),
        (app_id::SYSTEM_MONITOR, "System Monitor"),
        (app_id::APP_DRAWER, "Apps"),
        (app_id::SYSTEM_SETTINGS, "Settings"),
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
