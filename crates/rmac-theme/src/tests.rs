use super::*;
use crate::model::StoredPreferences;
use crate::store::event_targets_path;
use rmac_appearance::{
    AccentColor, ColorScheme, Contrast, MotionPreference, ResolvedColorScheme,
    Snapshot as HostSnapshot, TextScale,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn test_store(name: &str) -> (PathBuf, ThemeStore) {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "rmac-theme-{name}-{}-{sequence}",
        std::process::id()
    ));
    (root.clone(), ThemeStore::new(root.join("theme.json")))
}

fn host() -> HostSnapshot {
    HostSnapshot {
        available: true,
        color_scheme: ColorScheme::PreferDark,
        accent_color: AccentColor::new(0.8, 0.2, 0.4),
        contrast: Contrast::Higher,
        motion: MotionPreference::Reduced,
        ..HostSnapshot::default()
    }
}

#[test]
fn automatic_preferences_use_measured_blue_and_follow_other_host_values() {
    let resolved = Preferences::default().resolve(&host()).unwrap();
    assert_eq!(resolved.color_scheme, ResolvedColorScheme::Dark);
    assert_eq!(resolved.accent_color.components(), DEFAULT_ACCENT);
    assert_eq!(resolved.contrast, Contrast::Higher);
    assert_eq!(resolved.motion, MotionPreference::Reduced);
    assert_eq!(resolved.text_scale, TextScale::Standard);
}

#[test]
fn explicit_preferences_override_host_values() {
    let preferences = Preferences {
        color_scheme: SchemePreference::Light,
        accent_color: AccentPreference::Custom([0.1, 0.2, 0.3]),
        contrast: ContrastPreference::Normal,
        motion: MotionPreferenceSetting::Full,
        text_scale: TextScalePreference::ExtraLarge,
        allow_wallpaper_tinting: false,
    };
    let resolved = preferences.resolve(&host()).unwrap();
    assert_eq!(resolved.color_scheme, ResolvedColorScheme::Light);
    assert_eq!(resolved.accent_color.components(), (0.1, 0.2, 0.3));
    assert_eq!(resolved.contrast, Contrast::Normal);
    assert_eq!(resolved.motion, MotionPreference::Full);
    assert_eq!(resolved.text_scale, TextScale::ExtraLarge);
}

#[test]
fn save_round_trips_versioned_preferences() {
    let (root, store) = test_store("round-trip");
    let preferences = Preferences {
        color_scheme: SchemePreference::Dark,
        accent_color: AccentPreference::Custom([0.2, 0.4, 0.6]),
        contrast: ContrastPreference::Higher,
        motion: MotionPreferenceSetting::Reduced,
        text_scale: TextScalePreference::Large,
        allow_wallpaper_tinting: false,
    };
    store.save(&preferences, &host()).unwrap();
    let loaded = store.load(&host()).unwrap();
    assert_eq!(loaded.preferences, preferences);
    assert!(!loaded.recovered_from_last_good);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_one_documents_without_text_scale_keep_standard_size() {
    let stored: StoredPreferences =
        serde_json::from_str(r#"{"version":1,"preferences":{"color_scheme":"dark"}}"#).unwrap();
    assert_eq!(stored.preferences.text_scale, TextScalePreference::Standard);
    assert!(stored.preferences.allow_wallpaper_tinting);
}

#[test]
fn corrupt_primary_recovers_last_known_good_preferences() {
    let (root, store) = test_store("recovery");
    let preferences = Preferences {
        color_scheme: SchemePreference::Dark,
        ..Preferences::default()
    };
    store.save(&preferences, &host()).unwrap();
    std::fs::write(store.path(), b"not json").unwrap();
    let loaded = store.load(&host()).unwrap();
    assert_eq!(loaded.preferences, preferences);
    assert!(loaded.recovered_from_last_good);
    assert!(loaded.detail.unwrap().contains("primary file failed"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_accent_without_touching_the_store() {
    let (root, store) = test_store("invalid");
    let preferences = Preferences {
        accent_color: AccentPreference::Custom([0.0, f64::NAN, 1.0]),
        ..Preferences::default()
    };
    let error = store.save(&preferences, &host()).unwrap_err();
    assert_eq!(error.operation, Operation::ValidatePreferences);
    assert!(!store.path().exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_future_fields_are_tolerated_for_forward_compatibility() {
    let (root, store) = test_store("unknown-field");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        r#"{"version":1,"preferences":{"color_scheme":"dark","future":true}}"#,
    )
    .unwrap();
    let loaded = store.load(&host()).unwrap();
    assert_eq!(loaded.preferences.color_scheme, SchemePreference::Dark);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn watcher_filter_ignores_sibling_files_and_accepts_atomic_rename_targets() {
    let target = PathBuf::from("/tmp/rmac/theme.json");
    assert!(!event_targets_path(
        &[PathBuf::from("/tmp/rmac/theme.json.last-good")],
        &target
    ));
    assert!(event_targets_path(
        &[PathBuf::from("/tmp/rmac/.theme.json.tmp"), target.clone()],
        &target
    ));
}
