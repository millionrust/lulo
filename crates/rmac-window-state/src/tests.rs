use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn test_path(name: &str) -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rmac-window-state-{name}-{}-{sequence}/state.json",
        std::process::id()
    ))
}

#[test]
fn state_round_trips_through_private_atomic_store() {
    let path = test_path("round-trip");
    let store = Store::at(path.clone());
    let state = WindowState::checked(40.0, 80.0, 900.0, 640.0, WindowMode::Maximized).unwrap();

    assert_eq!(store.load().unwrap(), None);
    store.save(state).unwrap();
    assert_eq!(store.load().unwrap(), Some(state));

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn corrupt_primary_recovers_the_last_known_good_state() {
    let path = test_path("recovery");
    let store = Store::at(path.clone());
    let state = WindowState::checked(40.0, 80.0, 900.0, 640.0, WindowMode::Windowed).unwrap();
    store.save(state).unwrap();
    std::fs::write(&path, b"not json").unwrap();

    assert_eq!(store.load().unwrap(), Some(state));

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn disconnected_placement_moves_to_primary_and_preserves_mode() {
    let state = WindowState::checked(4000.0, 200.0, 1600.0, 1000.0, WindowMode::Maximized).unwrap();
    let primary = DisplayBounds::checked(0.0, 0.0, 1280.0, 720.0).unwrap();

    assert_eq!(
        state.fit_to_displays(&[primary], 640.0, 360.0),
        WindowState::checked(0.0, 0.0, 1280.0, 720.0, WindowMode::Maximized)
    );
}

#[test]
fn visible_placement_is_clamped_wholly_onto_its_display() {
    let state = WindowState::checked(1200.0, 650.0, 900.0, 640.0, WindowMode::Windowed).unwrap();
    let primary = DisplayBounds::checked(0.0, 0.0, 1280.0, 720.0).unwrap();

    assert_eq!(
        state.fit_to_displays(&[primary], 640.0, 360.0),
        WindowState::checked(380.0, 80.0, 900.0, 640.0, WindowMode::Windowed)
    );
}

#[test]
fn path_like_application_ids_are_rejected() {
    assert_eq!(
        validate_app_id("../org.rmac.Files").unwrap_err().operation,
        Operation::ResolvePath
    );
    assert!(validate_app_id("org.rmac.Files").is_ok());
}
