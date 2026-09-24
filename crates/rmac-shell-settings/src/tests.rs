//! Focused shared shell settings contracts.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn fresh_profile_has_a_deliberate_first_party_dock() {
    let settings = ShellSettings::default();
    assert!(!settings.dock.magnification);
    assert_eq!(
        settings.pinned_apps,
        [
            rmac_apps::identity::FILES,
            rmac_apps::identity::APP_DRAWER,
            rmac_apps::identity::NOTES,
            rmac_apps::identity::TEXT_EDITOR,
            rmac_apps::identity::TERMINAL,
            rmac_apps::identity::SYSTEM_SETTINGS,
        ]
        .into_iter()
        .map(|identity| AppId(identity.into()))
        .collect::<Vec<_>>()
    );
}

fn test_store(label: &str) -> (PathBuf, ShellSettingsStore) {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "rmac-shell-settings-{label}-{}-{sequence}",
        std::process::id()
    ));
    (
        root.clone(),
        ShellSettingsStore::new(root.join("shell.json")),
    )
}

fn settings() -> ShellSettings {
    ShellSettings {
        pinned_apps: vec![
            AppId("org.rmac.Files".into()),
            AppId("org.rmac.Terminal".into()),
        ],
        dock: DockSettings {
            placement: DockPlacement::Left,
            outputs: OutputScope::Named("DP-1".into()),
            autohide: true,
            magnification: true,
            magnification_scale: 1.8,
            reserve_space: false,
            repeated_click: RepeatedClickBehavior::HideApplication,
        },
        clock: ClockSettings {
            format: ClockFormat::TwentyFourHour,
            show_seconds: true,
            ..ClockSettings::default()
        },
        wallpaper: WallpaperSettings {
            default: WallpaperSelection {
                source: Some("file:///home/test/Pictures/wallpaper.jpg".into()),
                fit: WallpaperFit::Fill,
            },
            ..WallpaperSettings::default()
        },
        focus: FocusSettings {
            enabled: true,
            selected_mode: Some("work".into()),
            ends_at_unix_ms: Some(4_000_000_000_000),
        },
        providers: BTreeMap::from([(
            ProviderId("files".into()),
            ProviderPolicy {
                enabled: true,
                allow_private_content: true,
                allow_network: false,
            },
        )]),
        spotlight: SpotlightSettings {
            excluded_paths: vec!["/home/test/Private".into()],
            include_removable_mounts: true,
        },
        hot_corners: HotCornerSettings {
            top_left: HotCornerAction::MissionControl,
            bottom_right: HotCornerAction::Desktop,
            ..HotCornerSettings::default()
        },
        click_wallpaper_to_reveal: ClickWallpaperToReveal::Never,
        ..ShellSettings::default()
    }
}

#[test]
fn settings_round_trip_through_versioned_primary_and_last_good_files() {
    let (root, store) = test_store("round-trip");
    let expected = settings();
    store.save(&expected).unwrap();

    assert_eq!(store.load().unwrap().settings, expected);
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(stored["version"], CURRENT_VERSION);
    assert!(root.join("shell.json.last-good").exists());
    std::fs::remove_dir_all(root).unwrap();
}

/// The same round trip as
/// `settings_round_trip_through_versioned_primary_and_last_good_files`, but
/// through `rmac_storage::fake::InMemoryBackend` instead of the real filesystem —
/// the in-memory fake the store's `Backend` type parameter exists to allow,
/// so app tests don't have to touch disk.
#[test]
fn settings_round_trip_through_an_in_memory_backend() {
    use rmac_storage::fake::InMemoryBackend;

    let path = PathBuf::from("/state/shell.json");
    let store = ShellSettingsStore::with_backend(path, InMemoryBackend::new());
    let expected = settings();

    store.save(&expected).unwrap();
    assert_eq!(store.load().unwrap().settings, expected);
    assert_eq!(store.recovery_state(), RecoveryState::Current);
}

#[test]
fn v1_is_migrated_and_rewritten_without_losing_user_choices() {
    let (root, store) = test_store("migration");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        r#"{
                "version": 1,
                "pinned_apps": ["org.rmac.Finder"],
                "dock": {"placement":"right","autohide":true,"magnification":false},
                "wallpaper": "file:///home/test/old.jpg",
                "focus_mode": "quiet"
            }"#,
    )
    .unwrap();

    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.migrated_from, Some(1));
    assert_eq!(
        snapshot.settings.pinned_apps,
        vec![AppId("org.rmac.Files".into())]
    );
    assert_eq!(snapshot.settings.dock.placement, DockPlacement::Right);
    assert!(snapshot.settings.dock.autohide);
    assert!(!snapshot.settings.dock.magnification);
    assert_eq!(
        snapshot.settings.focus.selected_mode.as_deref(),
        Some("quiet")
    );
    let rewritten: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(rewritten["version"], CURRENT_VERSION);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v3_application_identity_is_migrated_without_duplicate_dock_items() {
    let (root, store) = test_store("v3-application-id");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        r#"{
                "version": 3,
                "settings": {
                    "pinned_apps": [
                        "org.rmac.Terminal",
                        "org.rmac.Finder",
                        "org.rmac.Files",
                        "org.example.Calendar"
                    ]
                }
            }"#,
    )
    .unwrap();

    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.migrated_from, Some(3));
    assert_eq!(
        snapshot.settings.pinned_apps,
        vec![
            AppId("org.rmac.Terminal".into()),
            AppId("org.rmac.Files".into()),
            AppId("org.example.Calendar".into()),
        ]
    );
    let rewritten: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(rewritten["version"], CURRENT_VERSION);
    assert_eq!(
        rewritten["settings"]["pinned_apps"],
        serde_json::json!([
            "org.rmac.Terminal",
            "org.rmac.Files",
            "org.example.Calendar"
        ])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v3_migration_does_not_hide_unrelated_duplicate_ids() {
    let (root, store) = test_store("v3-duplicate");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        r#"{
                "version": 3,
                "settings": {
                    "pinned_apps": ["org.rmac.Terminal", "org.rmac.Terminal"]
                }
            }"#,
    )
    .unwrap();

    let error = store.load().unwrap_err();
    assert_eq!(error.operation, Operation::ValidateSettings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_primary_recovers_last_known_good_settings() {
    let (root, store) = test_store("recovery");
    let expected = settings();
    store.save(&expected).unwrap();
    std::fs::write(store.path(), b"not json").unwrap();

    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.settings, expected);
    assert!(snapshot.recovered_from_last_good);
    assert!(snapshot.detail.unwrap().contains("primary file failed"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_recovery_preserves_rejected_bytes_and_restores_last_good() {
    let (root, store) = test_store("explicit-recovery");
    let expected = settings();
    store.save(&expected).unwrap();
    let rejected = b"not json";
    std::fs::write(store.path(), rejected).unwrap();

    assert_eq!(store.recovery_state(), RecoveryState::LastGoodAvailable);
    let snapshot = store.restore_last_good().unwrap();
    assert_eq!(snapshot.settings, expected);
    assert_eq!(store.recovery_state(), RecoveryState::Current);
    assert_eq!(
        std::fs::read(root.join("shell.json.rejected-before-restore")).unwrap(),
        rejected
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_recovery_refuses_to_replace_valid_settings() {
    let (root, store) = test_store("valid-recovery");
    store.save(&settings()).unwrap();

    let error = store.restore_last_good().unwrap_err();
    assert_eq!(error.operation, Operation::RestoreLastGood);
    assert_eq!(error.error_kind, io::ErrorKind::AlreadyExists);
    assert!(!root.join("shell.json.rejected-before-restore").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_settings_are_rejected_before_any_write() {
    let (root, store) = test_store("invalid");
    let mut invalid = settings();
    invalid.dock.magnification_scale = f32::NAN;
    let error = store.save(&invalid).unwrap_err();
    assert_eq!(error.operation, Operation::ValidateSettings);
    assert!(!store.path().exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_fields_are_tolerated_but_unknown_versions_are_not() {
    let (root, store) = test_store("future");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        r#"{"version":2,"settings":{"future":true,"dock":{"future":42}}}"#,
    )
    .unwrap();
    let migrated = store.load().unwrap();
    assert_eq!(migrated.settings, ShellSettings::default());
    assert_eq!(migrated.migrated_from, Some(2));

    std::fs::remove_file(root.join("shell.json.last-good")).unwrap();
    std::fs::write(store.path(), r#"{"version":99,"settings":{}}"#).unwrap();
    let error = store.load().unwrap_err();
    assert_eq!(error.operation, Operation::ParseSettings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn validation_rejects_duplicates_and_incoherent_focus_expiry() {
    let path = Path::new("shell.json");
    let mut duplicate = ShellSettings {
        pinned_apps: vec![AppId("same".into()), AppId("same".into())],
        ..ShellSettings::default()
    };
    assert!(validate(&duplicate, path).is_err());
    duplicate.pinned_apps.clear();
    duplicate.focus.ends_at_unix_ms = Some(1);
    assert!(validate(&duplicate, path).is_err());
    duplicate.focus.enabled = true;
    duplicate.focus.ends_at_unix_ms = None;
    assert!(validate(&duplicate, path).is_err());

    let mut invalid_exclusion = ShellSettings::default();
    invalid_exclusion.spotlight.excluded_paths = vec!["relative/private".into()];
    assert!(validate(&invalid_exclusion, path).is_err());
    invalid_exclusion.spotlight.excluded_paths =
        vec!["/home/test/Private".into(), "/home/test/Private".into()];
    assert!(validate(&invalid_exclusion, path).is_err());

    let mut invalid_wallpaper = ShellSettings::default();
    invalid_wallpaper.wallpaper.default.source = Some("https://example.com/wallpaper.jpg".into());
    assert!(validate(&invalid_wallpaper, path).is_err());
    invalid_wallpaper.wallpaper.default.source = Some("relative/wallpaper.png".into());
    assert!(validate(&invalid_wallpaper, path).is_err());
    invalid_wallpaper.wallpaper.default.source = Some("builtin:../lulo".into());
    assert!(validate(&invalid_wallpaper, path).is_err());
    invalid_wallpaper.wallpaper.default.source = Some("builtin:".into());
    assert!(validate(&invalid_wallpaper, path).is_err());

    // Every shipped built-in saves, not only the original Aurora.
    let mut built_in = ShellSettings::default();
    for id in ["lulo", "lulo-nocturne", "rmac-aurora", "rmac-tide"] {
        built_in.wallpaper.default.source = Some(format!("builtin:{id}"));
        assert!(validate(&built_in, path).is_ok(), "{id} was rejected");
    }
}

#[test]
fn watcher_filter_ignores_last_good_and_accepts_primary_replacement() {
    let target = PathBuf::from("/tmp/rmac/shell.json");
    assert!(!event_targets_path(
        &[PathBuf::from("/tmp/rmac/shell.json.last-good")],
        &target
    ));
    assert!(event_targets_path(
        &[PathBuf::from("/tmp/rmac/.shell.tmp"), target.clone()],
        &target
    ));
}

struct FailPrimaryWrite {
    primary: PathBuf,
}

impl Backend for FailPrimaryWrite {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        if path == self.primary {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected primary write failure",
            ))
        } else {
            rmac_storage::atomic_write(path, contents)
        }
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

#[test]
fn failed_primary_write_restores_the_previous_last_good_copy() {
    let (root, store) = test_store("rollback");
    let original = settings();
    store.save(&original).unwrap();
    let path = store.path().to_path_buf();
    let failing = ShellSettingsStore::with_backend(
        path.clone(),
        FailPrimaryWrite {
            primary: path.clone(),
        },
    );
    let mut changed = original.clone();
    changed.dock.placement = DockPlacement::Right;

    let error = failing.save(&changed).unwrap_err();
    assert_eq!(error.operation, Operation::SaveSettings);
    assert_eq!(error.error_kind, io::ErrorKind::PermissionDenied);
    let backup = path.with_file_name("shell.json.last-good");
    let stored: StoredSettings = serde_json::from_slice(&std::fs::read(backup).unwrap()).unwrap();
    assert_eq!(stored.settings, original);
    assert_eq!(store.load().unwrap().settings, original);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hot_corners_default_off_and_read_from_older_files() {
    assert_eq!(
        ShellSettings::default().hot_corners,
        HotCornerSettings::default()
    );
    assert_eq!(HotCornerSettings::default().top_left, HotCornerAction::None);
    assert!(serde_json::from_str::<HotCornerSettings>(r#"{"top_left":"quick-note"}"#).is_err());
    let parsed: HotCornerSettings =
        serde_json::from_str(r#"{"top_left":"application-windows"}"#).unwrap();
    assert_eq!(parsed.top_left, HotCornerAction::ApplicationWindows);
    assert_eq!(parsed.bottom_right, HotCornerAction::None);
    assert_eq!(HotCornerAction::ALL.len(), 7);
    assert_eq!(
        HotCornerAction::NotificationCenter.title(),
        "Notification Centre"
    );
}

#[test]
fn click_wallpaper_to_reveal_defaults_to_always_like_macos() {
    assert_eq!(
        ShellSettings::default().click_wallpaper_to_reveal,
        ClickWallpaperToReveal::Always
    );
    // A file saved before the setting existed reads as the default.
    let (root, store) = test_store("reveal-default");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        store.path(),
        format!(r#"{{"version":{CURRENT_VERSION},"settings":{{"pinned_apps":[]}}}}"#),
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().settings.click_wallpaper_to_reveal,
        ClickWallpaperToReveal::Always
    );
    std::fs::remove_dir_all(root).unwrap();

    assert_eq!(
        serde_json::to_value(ClickWallpaperToReveal::Never).unwrap(),
        "never"
    );
    assert!(serde_json::from_str::<ClickWallpaperToReveal>(r#""only-in-stage-manager""#).is_err());
    assert_eq!(
        ClickWallpaperToReveal::ALL.map(ClickWallpaperToReveal::title),
        ["Always", "Never"]
    );
}
