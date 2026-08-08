use gpui::WindowBounds;

use super::{app_id, window::window_options_for_app_with_bounds};

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
