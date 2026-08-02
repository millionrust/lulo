use std::sync::atomic::{AtomicU64, Ordering};

use rmac_focus::{ClockSample, Config, ModeId, Weekday};
use rmac_focus_store::{default_config, Store};

use super::*;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!(
            "rmac-focus-runtime-{}-{label}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
        .join("focus.json")
}

fn clock(unix_ms: u64, monotonic_ms: u64, minute: u16) -> ClockSample {
    ClockSample {
        unix_ms,
        monotonic_ms,
        weekday: Weekday::Monday,
        minute_of_day: minute,
        next_minute_unix_ms: unix_ms + 60_000,
    }
}

#[test]
fn mutation_persists_and_restart_expires_temporary_focus() {
    let path = path("restart");
    let store = Store::at(path.clone());
    let (mut runtime, _) = Runtime::load(store.clone(), clock(1_000, 1_000, 100)).unwrap();
    let update = runtime
        .activate_for(
            ModeId::parse("work").unwrap(),
            5_000,
            clock(1_000, 1_000, 100),
        )
        .unwrap();
    assert!(update.projection.enabled);
    assert_eq!(update.projection.ends_at_unix_ms, Some(6_000));
    drop(runtime);

    let (restarted, expired) = Runtime::load(store, clock(7_000, 2_000, 100)).unwrap();
    assert!(!expired.projection.enabled);
    assert!(restarted.manual_activation().is_none());
    assert_eq!(restarted.persistence_health(), PersistenceHealth::Healthy);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn replacing_config_drops_only_a_removed_manual_mode() {
    let path = path("replace");
    let store = Store::at(path.clone());
    let (mut runtime, _) = Runtime::load(store, clock(1_000, 1_000, 100)).unwrap();
    runtime
        .activate_indefinitely(ModeId::parse("work").unwrap(), clock(1_000, 1_000, 100))
        .unwrap();
    let do_not_disturb = default_config()
        .unwrap()
        .modes()
        .find(|mode| mode.id().as_str() == "do-not-disturb")
        .unwrap()
        .clone();
    let config = Config::new(vec![do_not_disturb], Vec::new()).unwrap();
    let update = runtime
        .replace_config(config, clock(2_000, 2_000, 100))
        .unwrap();
    assert!(!update.projection.enabled);
    assert!(runtime.manual_activation().is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn projection_and_debug_redact_mode_name() {
    let path = path("redaction");
    let store = Store::at(path.clone());
    let (mut runtime, _) = Runtime::load(store, clock(1_000, 1_000, 100)).unwrap();
    let update = runtime
        .activate_indefinitely(ModeId::parse("personal").unwrap(), clock(1_000, 1_000, 100))
        .unwrap();
    assert_eq!(update.projection.mode_name.as_deref(), Some("Personal"));
    assert!(!format!("{:?}", update.projection).contains("Personal"));
    assert_eq!(wake_delay(&update, 1_000), None);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn clock_sampler_returns_a_valid_local_minute_boundary() {
    let sample = ClockSampler::default().sample();
    assert!(sample.validate().is_ok());
    assert!(sample.next_minute_unix_ms > sample.unix_ms);
}
