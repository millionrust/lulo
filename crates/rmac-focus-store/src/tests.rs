use super::*;
use rmac_focus::{Config, ManualActivation, Mode, ModeId, Schedule, ScheduleId, Weekday};
use rmac_notifications::AppId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn path(label: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "rmac-focus-store-{}-{label}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
        .join("focus.json")
}

fn custom() -> (Config, ManualActivation) {
    let mode = Mode::new(
        ModeId::parse("private-work-8472").unwrap(),
        "Private Work 8472",
        [AppId::parse("org.private.App8472").unwrap()]
            .into_iter()
            .collect(),
        false,
    )
    .unwrap();
    let schedule = Schedule {
        id: ScheduleId::parse("private-schedule-8472").unwrap(),
        mode: mode.id().clone(),
        days: [Weekday::Friday].into_iter().collect(),
        start_minute: 22 * 60,
        end_minute: 7 * 60,
        priority: 4,
        enabled: true,
    };
    let config = Config::new(vec![mode], vec![schedule]).unwrap();
    let manual = ManualActivation {
        mode: ModeId::parse("private-work-8472").unwrap(),
        until_unix_ms: Some(9_000_000),
    };
    (config, manual)
}

#[test]
fn private_round_trip_preserves_modes_schedules_manual_and_permissions() {
    let path = path("roundtrip");
    let store = Store::at(path.clone());
    let (config, manual) = custom();
    store.save(&config, Some(&manual)).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.recovery, Recovery::None);
    assert_eq!(loaded.config, config);
    assert_eq!(loaded.manual, Some(manual));
    assert!(!format!("{store:?}").contains(path.to_string_lossy().as_ref()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn corrupt_primary_uses_last_good_and_redacts_debug() {
    let path = path("recovery");
    let store = Store::at(path.clone());
    let (config, manual) = custom();
    store.save(&config, Some(&manual)).unwrap();
    std::fs::write(&path, b"private corrupt data 8472").unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.recovery, Recovery::LastGood);
    let debug = format!("{loaded:?}");
    assert!(!debug.contains("8472"));
    assert!(!debug.contains("org.private"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn missing_or_double_corrupt_files_use_safe_defaults() {
    let path = path("defaults");
    let store = Store::at(path.clone());
    let missing = store.load().unwrap();
    assert_eq!(missing.recovery, Recovery::None);
    assert_eq!(missing.config.modes().count(), 4);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"bad").unwrap();
    std::fs::write(store.backup_path(), b"also bad").unwrap();
    let recovered = store.load().unwrap();
    assert_eq!(recovered.recovery, Recovery::Defaults);
    assert_eq!(recovered.config.modes().count(), 4);
    assert!(recovered.manual.is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn invalid_mode_references_fail_validation_and_recover() {
    let path = path("invalid-reference");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let invalid = serde_json::json!({
        "version": 1,
        "modes": [{
            "id": "known",
            "name": "Known",
            "allowed_apps": [],
            "allow_urgent": false
        }],
        "schedules": [{
            "id": "bad",
            "mode": "missing",
            "days": ["monday"],
            "start_minute": 10,
            "end_minute": 20,
            "priority": 1,
            "enabled": true
        }],
        "manual": null
    });
    std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    let loaded = Store::at(path.clone()).load().unwrap();
    assert_eq!(loaded.recovery, Recovery::Defaults);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
