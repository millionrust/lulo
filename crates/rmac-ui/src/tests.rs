use super::{app_id, window_options_for_app, window_options_unified_for_app};

#[test]
fn both_window_styles_publish_the_exact_application_id() {
    assert_eq!(
        window_options_for_app(app_id::TEXT_EDITOR, 800.0, 600.0).app_id,
        Some(app_id::TEXT_EDITOR.to_owned())
    );
    assert_eq!(
        window_options_unified_for_app(app_id::FILES, 800.0, 600.0).app_id,
        Some(app_id::FILES.to_owned())
    );
    assert_eq!(
        window_options_for_app(app_id::NOTES, 1080.0, 720.0).app_id,
        Some(app_id::NOTES.to_owned())
    );
}
