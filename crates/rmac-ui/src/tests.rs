use gpui::{SharedString, WindowBounds, WindowOptions};

use super::{app_id, window::window_options_for_app_with_bounds};

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
        (app_id::FILES, "Files"),
        (app_id::TERMINAL, "Terminal"),
        (app_id::NOTES, "Notes"),
        (app_id::TEXT_EDITOR, "Text Editor"),
        (app_id::SYSTEM_MONITOR, "System Monitor"),
        (app_id::APP_DRAWER, "Applications"),
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
