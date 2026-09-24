use super::*;
use crate::model::SAFE_MODE_VERSION;
use rmac_shell_settings::RecoveryState;
use std::io;
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::Output;
use std::sync::Mutex;

const HEALTHY: &str = "Id=rmac-dock.service\nLoadState=loaded\nActiveState=active\nSubState=running\nResult=success\nNRestarts=1\nMainPID=42\nExecMainStatus=0\n";

#[test]
fn parses_health_without_treating_pid_zero_as_a_process() {
    let health = parse_component_health(HEALTHY, "rmac-dock.service").unwrap();
    assert!(health.healthy());
    assert_eq!(health.main_pid, Some(42));

    let inactive = HEALTHY
        .replace("ActiveState=active", "ActiveState=inactive")
        .replace("SubState=running", "SubState=dead")
        .replace("MainPID=42", "MainPID=0");
    let health = parse_component_health(&inactive, "rmac-dock.service").unwrap();
    assert!(!health.healthy());
    assert_eq!(health.main_pid, None);
}

#[test]
fn diagnostics_omit_process_ids_paths_logs_and_settings_content() {
    let health = SessionHealth {
        observed_at_unix_ms: 42,
        safe_mode: Some(SafeModeState {
            version: SAFE_MODE_VERSION,
            entered_at_unix_ms: 41,
            trigger_unit: "rmac-dock.service".into(),
            observed_restarts: 3,
            reason: "/home/alice/private.txt token=secret".into(),
            trigger_executable: None,
        }),
        components: vec![parse_component_health(HEALTHY, "rmac-dock.service").unwrap()],
    };
    let report = DiagnosticReport::from_health(health, RecoveryState::LastGoodAvailable);
    let json = serde_json::to_string(&report).unwrap();

    assert!(json.contains("\"shell_settings_recovery\":\"last-good-available\""));
    assert!(json.contains("\"safe_mode_trigger_unit\":\"rmac-dock.service\""));
    for private in ["main_pid", "/home/alice", "private.txt", "token", "secret"] {
        assert!(!json.contains(private));
    }
}

#[test]
fn rejects_missing_fields_and_mismatched_units() {
    assert!(parse_component_health("Id=rmac-dock.service\n", "rmac-dock.service").is_err());
    assert!(parse_component_health(HEALTHY, "rmac-top-bar.service").is_err());
}

struct FakeRunner {
    outputs: Mutex<Vec<Output>>,
}

impl CommandRunner for FakeRunner {
    fn output(&self, _: &str, _: &[&str]) -> io::Result<Output> {
        Ok(self.outputs.lock().unwrap().remove(0))
    }
}

fn output(text: &str) -> Output {
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: text.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn paths(label: &str) -> (PathBuf, StatePaths) {
    let root = std::env::temp_dir().join(format!("rmac-session-{label}-{}", std::process::id()));
    let paths = StatePaths {
        safe_mode: root.join("state/safe-mode.json"),
        last_safe_mode: root.join("state/safe-mode.last.json"),
        safe_login: root.join("runtime/safe-mode-login.json"),
        health: root.join("runtime/health.json"),
        libexec: root.join("libexec"),
    };
    (root, paths)
}

fn exhausted_dock() -> String {
    HEALTHY
        .replace("Result=success", "Result=exit-code")
        .replace("NRestarts=1", "NRestarts=3")
}

fn supervisor_with(paths: StatePaths, outputs: Vec<Output>) -> Supervisor<FakeRunner> {
    Supervisor::new(
        paths,
        FakeRunner {
            outputs: Mutex::new(outputs),
        },
    )
}

fn install_executable(paths: &StatePaths, name: &str, contents: &str) {
    std::fs::create_dir_all(&paths.libexec).unwrap();
    let path = paths.libexec.join(name);
    // Replace, as package and development installs do, so the inode changes.
    let _ = std::fs::remove_file(&path);
    std::fs::write(path, contents).unwrap();
}

fn last_record(paths: &StatePaths) -> SafeModeRecord {
    serde_json::from_slice(&std::fs::read(&paths.last_safe_mode).unwrap()).unwrap()
}

#[test]
fn safe_mode_lasts_exactly_one_login() {
    let (root, paths) = paths("one-shot");
    install_executable(&paths, "rmac-dock", "build 1");
    let supervisor = supervisor_with(paths.clone(), vec![output(&exhausted_dock())]);
    assert!(supervisor.observe_failure("rmac-dock.service").unwrap());
    let recorded = supervisor.load_safe_mode().unwrap().unwrap();
    assert!(recorded.trigger_executable.is_some());

    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Safe);
    assert!(
        !paths.safe_mode.exists(),
        "the safe login consumes the marker"
    );
    let record = last_record(&paths);
    assert_eq!(record.outcome, SafeModeOutcome::SafeLogin);
    assert_eq!(record.state.as_ref(), Some(&recorded));
    assert_eq!(supervisor.load_safe_login().unwrap(), Some(record));
    assert_eq!(
        supervisor.notice_context().unwrap(),
        Some((Some("rmac-dock.service".into()), NoticeContext::ThisLogin))
    );

    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Normal);
    assert_eq!(supervisor.load_safe_login().unwrap(), None);
    assert_eq!(supervisor.notice_context().unwrap(), None);
    assert_eq!(last_record(&paths).outcome, SafeModeOutcome::SafeLogin);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_rebuilt_component_skips_safe_mode_recorded_against_the_old_build() {
    let (root, paths) = paths("rebuilt");
    install_executable(&paths, "rmac-dock", "build 1");
    let supervisor = supervisor_with(paths.clone(), vec![output(&exhausted_dock())]);
    assert!(supervisor.observe_failure("rmac-dock.service").unwrap());

    install_executable(&paths, "rmac-dock", "build 2, fixed");
    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Normal);
    assert!(!paths.safe_mode.exists());
    assert!(!paths.safe_login.exists());
    assert_eq!(
        last_record(&paths).outcome,
        SafeModeOutcome::SkippedChangedExecutable
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failure_during_a_session_is_announced_as_pending_for_the_next_login() {
    let (root, paths) = paths("during-session");
    let supervisor = supervisor_with(paths, vec![output(&exhausted_dock())]);
    assert!(supervisor.observe_failure("rmac-dock.service").unwrap());
    assert_eq!(
        supervisor.notice_context().unwrap(),
        Some((
            Some("rmac-dock.service".into()),
            NoticeContext::DuringSession
        ))
    );
    supervisor.clear_safe_mode().unwrap();
    assert_eq!(supervisor.notice_context().unwrap(), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unreadable_marker_cannot_hold_every_login_in_safe_mode() {
    let (root, paths) = paths("invalid-marker");
    std::fs::create_dir_all(paths.safe_mode.parent().unwrap()).unwrap();
    std::fs::write(&paths.safe_mode, b"{not json").unwrap();
    let supervisor = supervisor_with(paths.clone(), vec![]);
    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Normal);
    assert!(!paths.safe_mode.exists());
    let record = last_record(&paths);
    assert_eq!(record.outcome, SafeModeOutcome::DiscardedInvalidMarker);
    assert_eq!(record.state, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn markers_from_before_build_identity_are_honoured_once() {
    let (root, paths) = paths("legacy-marker");
    std::fs::create_dir_all(paths.safe_mode.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.safe_mode,
        br#"{"version":1,"entered_at_unix_ms":1,"trigger_unit":"rmac-osd.service","observed_restarts":3,"reason":"start limit"}"#,
    )
    .unwrap();
    let supervisor = supervisor_with(paths.clone(), vec![]);
    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Safe);
    assert_eq!(supervisor.begin_login().unwrap(), LoginMode::Normal);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn login_outcome_compares_the_recorded_and_installed_builds() {
    let identity = ExecutableIdentity {
        path: "/usr/libexec/rmac/rmac-dock".into(),
        device: 1,
        inode: 2,
        size: 3,
        modified_unix_s: 4,
        modified_nsec: 5,
    };
    let mut state = SafeModeState {
        version: SAFE_MODE_VERSION,
        entered_at_unix_ms: 1,
        trigger_unit: "rmac-dock.service".into(),
        observed_restarts: 3,
        reason: "budget".into(),
        trigger_executable: None,
    };
    assert_eq!(login_outcome(&state, None), SafeModeOutcome::SafeLogin);
    state.trigger_executable = Some(identity.clone());
    assert_eq!(
        login_outcome(&state, Some(&identity)),
        SafeModeOutcome::SafeLogin
    );
    let reinstalled = ExecutableIdentity {
        inode: 9,
        ..identity.clone()
    };
    assert_eq!(
        login_outcome(&state, Some(&reinstalled)),
        SafeModeOutcome::SkippedChangedExecutable
    );
    assert_eq!(
        login_outcome(&state, None),
        SafeModeOutcome::SkippedChangedExecutable,
        "an uninstalled component cannot fail again"
    );
}

#[test]
fn the_safe_mode_notice_names_the_component_and_what_happens_next() {
    let login = safe_mode_notice(Some("rmac-osd.service"), NoticeContext::ThisLogin);
    assert!(login
        .body
        .starts_with("The volume and brightness display quit"));
    assert!(login.body.contains("next login will start normally"));
    assert_eq!(login.action_label, "Restart Normally");

    let live = safe_mode_notice(Some("rmac-dock.service"), NoticeContext::DuringSession);
    assert!(live.body.starts_with("The Dock quit"));
    assert!(live.body.contains("unless you restart normally now"));

    let unknown = safe_mode_notice(None, NoticeContext::ThisLogin);
    assert!(unknown.body.starts_with("An rmac component quit"));
}

#[test]
fn component_executables_match_the_unit_files() {
    let units = [
        (
            "rmac-top-bar.service",
            include_str!("../units/rmac-top-bar.service"),
        ),
        (
            "rmac-dock.service",
            include_str!("../units/rmac-dock.service"),
        ),
        (
            "rmac-launcher.service",
            include_str!("../units/rmac-launcher.service"),
        ),
        (
            "rmac-quick-settings.service",
            include_str!("../units/rmac-quick-settings.service"),
        ),
        (
            "rmac-notification-center.service",
            include_str!("../units/rmac-notification-center.service"),
        ),
        (
            "rmac-notification-center-panel.service",
            include_str!("../units/rmac-notification-center-panel.service"),
        ),
        (
            "rmac-focus.service",
            include_str!("../units/rmac-focus.service"),
        ),
        (
            "rmac-wallpaper.service",
            include_str!("../units/rmac-wallpaper.service"),
        ),
        (
            "rmac-osd.service",
            include_str!("../units/rmac-osd.service"),
        ),
        (
            "rmac-app-switcher.service",
            include_str!("../units/rmac-app-switcher.service"),
        ),
        (
            "rmac-screenshot.service",
            include_str!("../units/rmac-screenshot.service"),
        ),
        (
            "rmac-mission-control.service",
            include_str!("../units/rmac-mission-control.service"),
        ),
        (
            "rmac-clipboard.service",
            include_str!("../units/rmac-clipboard.service"),
        ),
        (
            "rmac-shortcut-broker.service",
            include_str!("../units/rmac-shortcut-broker.service"),
        ),
    ];
    assert_eq!(units.len(), COMPONENT_UNITS.len());
    for (unit, text) in units {
        assert!(COMPONENT_UNITS.contains(&unit));
        let executable = component_executable(unit).unwrap();
        let exec_start = format!("ExecStart=%h/.local/libexec/rmac/{executable}");
        assert!(
            text.lines()
                .any(|line| line == exec_start || line.starts_with(&format!("{exec_start} "))),
            "{unit} does not start {executable}"
        );
    }
    assert_eq!(component_executable("rmac-lock.service"), None);
}

#[test]
fn the_safe_mode_notice_unit_runs_only_in_safe_mode() {
    let notice = include_str!("../units/rmac-safe-mode-notice.service");
    assert!(notice
        .contains("ExecStart=%h/.local/libexec/rmac/rmac-session-supervisor notify-safe-mode"));
    assert!(notice.contains("PartOf=rmac-safe-mode.target"));
    assert!(!notice.contains("OnFailure="));
    assert!(!notice.contains("/bin/sh"));
    let safe_target = include_str!("../units/rmac-safe-mode.target");
    assert!(safe_target.contains("rmac-safe-mode-notice.service"));
    let normal_target = include_str!("../units/rmac-session.target");
    assert!(!normal_target.contains("rmac-safe-mode-notice.service"));
}

#[test]
fn the_supervisor_waits_for_events_instead_of_polling() {
    let main = include_str!("main.rs");
    assert!(!main.contains("Duration::from_secs(5)"));
    assert!(main.contains("recv_timeout(RECONCILE_INTERVAL)"));
    assert!(main.contains("\"JobRemoved\""));
}

#[test]
fn bounded_failure_enters_persistent_safe_mode() {
    let (root, paths) = paths("safe-mode");
    let failed = HEALTHY
        .replace("Result=success", "Result=exit-code")
        .replace("NRestarts=1", "NRestarts=3");
    let supervisor = Supervisor::new(
        paths,
        FakeRunner {
            outputs: Mutex::new(vec![output(&failed)]),
        },
    );

    assert!(supervisor.observe_failure("rmac-dock.service").unwrap());
    let state = supervisor.load_safe_mode().unwrap().unwrap();
    assert_eq!(state.trigger_unit, "rmac-dock.service");
    assert_eq!(state.observed_restarts, 3);
    supervisor.clear_safe_mode().unwrap();
    assert_eq!(supervisor.load_safe_mode().unwrap(), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn nonessential_failure_stays_contained_without_safe_mode() {
    let (root, paths) = paths("contained");
    let exhausted = HEALTHY
        .replace("rmac-dock.service", "rmac-osd.service")
        .replace("Result=success", "Result=start-limit-hit")
        .replace("NRestarts=1", "NRestarts=3");
    let mut outputs = vec![output(&exhausted)];
    outputs.extend(
        COMPONENT_UNITS
            .iter()
            .map(|unit| output(&HEALTHY.replace("rmac-dock.service", unit))),
    );
    let supervisor = Supervisor::new(
        paths,
        FakeRunner {
            outputs: Mutex::new(outputs),
        },
    );

    assert!(!supervisor.observe_failure("rmac-osd.service").unwrap());
    assert_eq!(supervisor.load_safe_mode().unwrap(), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unrecognized_units_before_running_a_command() {
    let (_, paths) = paths("unrecognized");
    let supervisor = Supervisor::new(
        paths,
        FakeRunner {
            outputs: Mutex::new(vec![]),
        },
    );
    let error = supervisor
        .component_health("malicious.service")
        .unwrap_err();
    assert_eq!(error.operation, Operation::ParseSystemctl);
}

#[test]
fn unit_assets_bound_restarts_and_keep_components_in_separate_crash_domains() {
    let resident_units = [
        include_str!("../units/rmac-top-bar.service"),
        include_str!("../units/rmac-dock.service"),
        include_str!("../units/rmac-notification-center.service"),
        include_str!("../units/rmac-focus.service"),
        include_str!("../units/rmac-wallpaper.service"),
        include_str!("../units/rmac-osd.service"),
        include_str!("../units/rmac-app-switcher.service"),
        include_str!("../units/rmac-screenshot.service"),
        include_str!("../units/rmac-mission-control.service"),
        include_str!("../units/rmac-clipboard.service"),
        include_str!("../units/rmac-shortcut-broker.service"),
    ];
    for unit in resident_units {
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartSec=1s"));
        assert!(unit.contains("StartLimitIntervalSec=60s"));
        assert!(unit.contains("StartLimitBurst=4"));
        assert!(unit.contains("OnFailure=rmac-component-failure@%N.service"));
        assert!(unit.contains("ConditionFileIsExecutable=%h/.local/libexec/rmac/"));
        assert!(!unit.contains("/bin/sh"));
    }
    let on_demand_units = [
        include_str!("../units/rmac-launcher.service"),
        include_str!("../units/rmac-app-drawer.service"),
        include_str!("../units/rmac-quick-settings.service"),
        include_str!("../units/rmac-notification-center-panel.service"),
    ];
    for unit in on_demand_units {
        assert!(unit.contains("Restart=on-success"));
        assert!(unit.contains("RestartSec=100ms"));
        assert!(unit.contains("StartLimitIntervalSec=0"));
        assert!(unit.contains("OnFailure=rmac-component-failure@%N.service"));
        assert!(unit.contains("ConditionFileIsExecutable=%h/.local/libexec/rmac/"));
        assert!(!unit.contains("Restart=on-failure"));
        assert!(!unit.contains("StartLimitBurst="));
        assert!(!unit.contains("/bin/sh"));
    }
    let failure = include_str!("../units/rmac-component-failure@.service");
    assert!(failure.contains("observe-failure %i.service"));
    assert!(!failure.contains("%I.service"));
    let notifications = include_str!("../units/rmac-notification-center.service");
    assert!(notifications.contains("Type=dbus"));
    assert!(notifications.contains("BusName=org.freedesktop.impl.portal.desktop.rmac"));
    assert!(notifications.contains("Before=xdg-desktop-portal.service"));
    let focus = include_str!("../units/rmac-focus.service");
    assert!(focus.contains("Type=dbus"));
    assert!(focus.contains("BusName=org.rmac.Focus1"));
    let clipboard = include_str!("../units/rmac-clipboard.service");
    assert!(clipboard.contains("Type=dbus"));
    assert!(clipboard.contains("BusName=org.rmac.Clipboard1"));
    let launcher = include_str!("../units/rmac-launcher.service");
    let app_drawer = include_str!("../units/rmac-app-drawer.service");
    let quick_settings = include_str!("../units/rmac-quick-settings.service");
    let notification_panel = include_str!("../units/rmac-notification-center-panel.service");
    let shortcut_broker = include_str!("../units/rmac-shortcut-broker.service");
    assert!(launcher.contains("Before=rmac-shortcut-broker.service"));
    assert!(launcher.contains("Type=notify"));
    assert!(launcher.contains("NotifyAccess=all"));
    assert!(app_drawer.contains("Before=rmac-shortcut-broker.service"));
    assert!(app_drawer.contains("Type=notify"));
    assert!(app_drawer.contains("NotifyAccess=all"));
    assert!(app_drawer.contains("ExecStart=%h/.local/libexec/rmac/rmac-app-drawer --service"));
    assert!(quick_settings.contains("Before=rmac-shortcut-broker.service"));
    assert!(quick_settings.contains("Type=notify"));
    assert!(quick_settings.contains("NotifyAccess=all"));
    assert!(notification_panel
        .contains("After=rmac-session-supervisor.service rmac-notification-center.service"));
    assert!(notification_panel.contains("Before=rmac-shortcut-broker.service"));
    assert!(notification_panel.contains("Type=notify"));
    assert!(notification_panel.contains("NotifyAccess=all"));
    assert!(shortcut_broker.contains("rmac-quick-settings.service"));
    assert!(!shortcut_broker.contains("rmac-app-drawer.service"));
    assert!(shortcut_broker.contains("rmac-notification-center-panel.service"));

    let lock = include_str!("../units/rmac-lock.service");
    assert!(lock.contains("Type=notify"));
    assert!(lock.contains("NotifyAccess=all"));
    assert!(lock.contains("Restart=on-failure"));
    assert!(lock.contains("RestartSec=1s"));
    assert!(lock.contains("StartLimitIntervalSec=30s"));
    assert!(lock.contains("StartLimitBurst=5"));
    assert!(lock.contains("OnFailure=rmac-lock-fallback.service"));
    assert!(lock.contains("ExecStart=%h/.local/libexec/rmac/rmac-lock-provider"));
    assert!(lock.contains("WatchdogSec=10s"));
    assert!(lock.contains("KillMode=control-group"));
    // A skipped (Condition) unit makes `systemctl start` succeed with
    // nothing locked; only an Assert fails the lock request.
    assert!(lock.contains("AssertPathExists=/etc/pam.d/rmac-lock"));
    assert!(!lock.contains("ConditionPathExists"));
    assert!(lock.contains("LimitCORE=0"));
    assert!(!lock.contains("OnFailure=rmac-component-failure"));
    assert!(!lock.contains("NoNewPrivileges=yes"));
    assert!(!lock.contains("/bin/sh"));

    let lock_fallback = include_str!("../units/rmac-lock-fallback.service");
    assert!(lock_fallback.contains("StartLimitIntervalSec=0"));
    assert!(lock_fallback.contains("Type=notify"));
    assert!(lock_fallback.contains(
        "ExecStart=%h/.local/libexec/rmac/rmac-locker --config %h/.config/rmac/swaylock.conf"
    ));
    assert!(!lock_fallback.contains("rmac-lock-provider"));

    let coordinator = include_str!("../units/rmac-lock-coordinator.service");
    assert!(coordinator.contains("Type=notify"));
    assert!(coordinator.contains("NotifyAccess=all"));
    assert!(coordinator.contains("After=graphical-session-pre.target"));
    assert!(!coordinator.contains("After=graphical-session.target"));
    assert!(coordinator.contains("Restart=on-failure"));
    assert!(coordinator.contains("StartLimitIntervalSec=0"));
    assert!(coordinator.contains("NoNewPrivileges=yes"));
    assert!(coordinator.contains(
        "ExecStart=%h/.local/libexec/rmac/rmac-lock-coordinator --policy %h/.config/rmac/lock-policy.json"
    ));
    assert!(!coordinator.contains("OnFailure=rmac-component-failure"));
    assert!(!coordinator.contains("/bin/sh"));

    let normal_target = include_str!("../units/rmac-session.target");
    let safe_target = include_str!("../units/rmac-safe-mode.target");
    assert!(normal_target
        .contains("Requires=rmac-session-supervisor.service rmac-lock-coordinator.service"));
    assert!(normal_target.contains("rmac-notification-center-panel.service"));
    assert!(normal_target.contains("rmac-app-drawer.service"));
    assert!(normal_target.contains("rmac-osd.service"));
    assert!(normal_target.contains("rmac-app-switcher.service"));
    assert!(normal_target.contains("rmac-screenshot.service"));
    assert!(normal_target.contains("rmac-mission-control.service"));
    assert!(normal_target.contains("rmac-clipboard.service"));
    assert!(safe_target
        .contains("Requires=rmac-session-supervisor.service rmac-lock-coordinator.service"));

    let idle_lock = include_str!("../units/rmac-idle-lock.service");
    assert!(idle_lock.contains("After=rmac-lock-coordinator.service"));
    assert!(idle_lock.contains("Restart=on-failure"));
    assert!(idle_lock.contains("StartLimitIntervalSec=0"));
    assert!(idle_lock.contains(
        "ExecStart=%h/.local/libexec/rmac/rmac-idle-locker --policy %h/.config/rmac/lock-policy.json"
    ));
    assert!(!idle_lock.contains("/bin/sh"));
    assert!(normal_target.contains("rmac-idle-lock.service"));
    assert!(safe_target.contains("Wants=rmac-idle-lock.service"));

    let mac_keyboard = include_str!("../units/rmac-mac-keyboard.service");
    assert!(mac_keyboard.contains("ConditionPathExists=/etc/keyd/rmac.conf"));
    assert!(mac_keyboard.contains("ExecStart=%h/.local/libexec/rmac/rmac-mac-keyboard follow"));
    assert!(mac_keyboard.contains("ExecStopPost=-%h/.local/libexec/rmac/rmac-mac-keyboard reset"));
    assert!(mac_keyboard.contains("Restart=on-failure"));
    assert!(!mac_keyboard.contains("/bin/sh"));
    assert!(normal_target.contains("rmac-mac-keyboard.service"));
    assert!(!safe_target.contains("rmac-mac-keyboard.service"));

    let setup = include_str!("../units/rmac-setup-assistant.service");
    assert!(setup.contains("ConditionPathExists=!%E/rmac/setup-assistant-complete"));
    assert!(setup.contains("ExecStart=%h/.local/libexec/rmac/rmac-setup-assistant --first-login"));
    assert!(setup.contains("Restart=no"));
    assert!(normal_target.contains("rmac-setup-assistant.service"));
    assert!(!safe_target.contains("rmac-setup-assistant.service"));

    let default_policy = include_str!("../lock-policy.json");
    assert!(default_policy.contains("\"version\": 1"));
    assert!(default_policy.contains("\"lock_after_seconds\": 300"));
    assert!(default_policy.contains("\"suspend_after_seconds\": null"));
}

/// Regression coverage for a real incident: rmac-quick-settings,
/// rmac-launcher, rmac-app-drawer, and rmac-notification-center-panel all
/// panicked with `NoCompositor` after being respawned (their units'
/// `Restart=on-success`) in the brief window between niri exiting during
/// logout/session-switch and the session's own `stop_rmac` shell-script
/// cleanup catching up. `PartOf=rmac-session.target` on those component
/// units only stops them once something actually stops
/// `rmac-session.target` -- it does not itself react to niri exiting.
/// Binding the session target directly to `niri.service` closes that race:
/// systemd stops the whole `PartOf=` tree the moment niri's own unit
/// deactivates, without waiting on the wrapper script's `wait` loop.
#[test]
fn the_session_target_stops_the_instant_niri_does_not_only_when_told_to() {
    let normal_target = include_str!("../units/rmac-session.target");
    assert!(normal_target.contains("BindsTo=graphical-session.target niri.service"));
    assert!(normal_target.contains("After=graphical-session-pre.target niri.service"));
}

#[test]
fn apps_and_spotlight_have_exactly_one_activation_owner_each() {
    let launcher = include_str!("../../launcher-app/src/service.rs");
    let app_drawer = include_str!("../../app-drawer/src/service.rs");

    assert!(launcher.contains("const LINUX_SHORTCUT_ENDPOINT: &str = \"launcher\";"));
    assert!(!launcher.contains("const LINUX_SHORTCUT_ENDPOINT: &str = \"app-drawer\";"));
    assert!(app_drawer.contains("const LINUX_SHORTCUT_ENDPOINT: &str = \"app-drawer\";"));
    assert!(!app_drawer.contains("const LINUX_SHORTCUT_ENDPOINT: &str = \"launcher\";"));
    assert_eq!(
        launcher
            .matches("rmac_shell_activation_runtime::watch(")
            .count(),
        1
    );
    assert_eq!(
        app_drawer
            .matches("rmac_shell_activation_runtime::watch(")
            .count(),
        1
    );
}

#[test]
fn desktop_overlays_are_linux_layer_surfaces_without_window_chrome() {
    let overlays = [
        (
            include_str!("../../launcher-app/src/service/overlay.rs"),
            "namespace: \"rmac-launcher\"",
        ),
        (
            include_str!("../../quick-settings-app/src/main.rs"),
            "namespace: rmac_quick_settings::surface::NAMESPACE",
        ),
        (
            include_str!("../../notification-center-app/src/main.rs"),
            "namespace: rmac_notifications_linux::center_surface::NAMESPACE",
        ),
        (
            include_str!("../../app-drawer/src/service.rs"),
            "namespace: \"rmac-app-drawer\"",
        ),
    ];

    for (source, namespace_declaration) in overlays {
        let linux_options = source
            .split_once("#[cfg(target_os = \"linux\")]")
            .expect("overlay has Linux-specific window options")
            .1
            .split("#[cfg(not(target_os = \"linux\"))]")
            .next()
            .expect("Linux window options precede the non-Linux fallback");
        assert!(linux_options.contains("WindowKind::LayerShell"));
        assert!(linux_options.contains("Layer::Overlay"));
        assert!(linux_options.contains(namespace_declaration));
        assert!(!linux_options.contains("WindowDecorations::Client"));
    }
}

#[test]
fn compositor_shortcuts_preserve_standard_command_keys() {
    let shell = include_str!("../../../packaging/rmac-session/shell.kdl");
    let fallback = include_str!("../../../packaging/rmac-session/shortcuts-fallback.kdl");

    for reserved in [
        "Mod+A ", "Mod+F ", "Mod+H ", "Mod+J ", "Mod+K ", "Mod+L ", "Mod+N ", "Mod+O ", "Mod+Q ",
        "Mod+T ", "Mod+W ",
    ] {
        assert!(!shell.contains(reserved), "shell captures {reserved}");
        assert!(!fallback.contains(reserved), "fallback captures {reserved}");
    }
    assert!(!shell.contains("shortcuts-fallback.kdl"));
    assert!(shell.contains("Ctrl+Up repeat=false hotkey-overlay-title=\"Mission Control\""));
    assert!(shell.contains("Mod+Ctrl+F repeat=false hotkey-overlay-title=\"Full Screen\""));
    // ⌘Tab is the per-application rmac switcher, not niri's window MRU.
    assert!(!shell.contains("Mod+Tab { next-window; }"));
    assert!(shell.contains(
        "Mod+Tab hotkey-overlay-title=\"Switch Applications\" { spawn \"/usr/libexec/rmac/rmac-app-switcher\" \"next\"; }"
    ));
    assert!(shell.contains("{ spawn \"/usr/libexec/rmac/rmac-app-switcher\" \"previous\"; }"));
    assert!(shell.contains("Mod+grave { next-window filter=\"app-id\"; }"));
    // ⌃F3 asks the resident Dock to take keyboard focus.
    assert!(shell.contains(
        "Ctrl+F3 repeat=false hotkey-overlay-title=\"Move Focus to the Dock\" { spawn \"/usr/libexec/rmac/rmac-dock\" \"focus\"; }"
    ));
    // ⌥⌘⎋ opens Force Quit, the same target as the system menu's item.
    assert!(shell.contains(
        "Mod+Alt+Escape repeat=false hotkey-overlay-title=\"Force Quit Applications\" { spawn \"/usr/libexec/rmac/rmac-app-switcher\" \"force-quit\"; }"
    ));
    // ⇧⌘3/4/5 go to the resident screenshot service, not niri's own UI.
    for (keys, word) in [
        ("Mod+Shift+3", "screen"),
        ("Mod+Shift+4", "selection"),
        ("Mod+Shift+5", "toolbar"),
    ] {
        assert!(shell.lines().any(|line| {
            line.trim_start().starts_with(keys)
                && line.contains(&format!(
                    "{{ spawn \"/usr/libexec/rmac/rmac-screenshot\" \"{word}\"; }}"
                ))
        }));
    }
    assert!(!shell.contains("\"screenshot-screen\""));
    // ⌃↑ / ⌃↓ / F11 / ⌃← / ⌃→ go to rmac's Mission Control service
    // (docs/decisions/0014), not niri's overview or raw workspace moves.
    for (keys, word) in [
        ("Ctrl+Up", "mission-control"),
        ("Ctrl+Down", "app-windows"),
        ("F11", "show-desktop"),
        ("Ctrl+Left", "previous-space"),
        ("Ctrl+Right", "next-space"),
    ] {
        assert!(shell.lines().any(|line| {
            line.trim_start().starts_with(keys)
                && line.contains(&format!(
                    "{{ spawn \"/usr/libexec/rmac/rmac-mission-control\" \"{word}\"; }}"
                ))
        }));
    }
    assert!(!shell.contains("toggle-overview"));
    assert!(fallback.contains("Mod+Space repeat=false"));
    assert!(fallback.contains("Mod+Ctrl+Q repeat=false allow-when-locked=true"));
    assert_eq!(fallback.matches("{ spawn ").count(), 2);
}

#[test]
fn every_window_uses_the_measured_radius_and_active_inactive_shadows() {
    let shell = include_str!("../../../packaging/rmac-session/shell.kdl");
    let rule = shell
        .split_once("// macOS uses freely movable, overlapping application windows.")
        .map(|(_, suffix)| suffix)
        .and_then(|suffix| suffix.split_once("\n}\n").map(|(rule, _)| rule))
        .expect("global application window rule");

    // This rule deliberately has no app-id matcher: Files, Settings, Terminal,
    // Notes, System Monitor, Text Editor, and third-party app IDs all receive
    // the same compositor-owned window silhouette.
    assert!(!rule.contains("match app-id="));
    assert!(rule.contains("geometry-corner-radius 16"));
    assert!(rule.contains("clip-to-geometry true"));
    // Inactive shadow on every window; the key window's deeper one below.
    assert!(rule.contains("softness 26"));
    assert!(rule.contains("spread 0"));
    assert!(rule.contains("offset x=0 y=8"));
    assert!(rule.contains("inactive-color \"#00000075\""));
    assert!(shell.contains(
        "match is-focused=true\n    shadow {\n        softness 42\n        offset x=0 y=16\n        color \"#000000bd\""
    ));
    // Unified-toolbar apps take the 27 pt toolbar-window radius; Text Editor
    // is a title-bar window like TextEdit and keeps the 16 pt default.
    assert!(shell.contains("|SystemMonitor)$\"#\n    geometry-corner-radius 27"));
    assert!(!shell.contains("TextEditor)$\"#\n    geometry-corner-radius 27"));
    // Header-bar apps under the rmac GTK theme are toolbar windows as well.
    assert!(shell.contains(
        "match app-id=r#\"^org\\.gnome\\.\"#\n    match app-id=r#\"^(firefox|org\\.mozilla\\.firefox|google-chrome|chromium|chromium-browser)$\"#\n    geometry-corner-radius 27"
    ));
}

#[test]
fn notification_portal_assets_select_only_the_rmac_backend_interface() {
    let descriptor = include_str!("../../rmac-notifications-linux/install/rmac.portal");
    assert!(descriptor.contains("DBusName=org.freedesktop.impl.portal.desktop.rmac"));
    assert!(descriptor.contains("Interfaces=org.freedesktop.impl.portal.Notification;"));
    assert!(descriptor.contains("UseIn=rmac"));

    let selection = include_str!("../../rmac-notifications-linux/install/rmac-portals.conf");
    assert!(selection.contains("default=gnome;gtk;*"));
    assert!(selection.contains("org.freedesktop.impl.portal.Notification=rmac"));
    // The notification process never claims FileChooser; the separate
    // rmac-file-chooser backend does, with GNOME as the fallback.
    assert!(!selection.contains("org.freedesktop.impl.portal.FileChooser=rmac\n"));
    assert!(
        selection.contains("org.freedesktop.impl.portal.FileChooser=rmac-file-chooser;gnome;gtk")
    );
    assert!(!descriptor.contains("FileChooser"));

    let activation = include_str!(
        "../../rmac-notifications-linux/install/org.freedesktop.impl.portal.desktop.rmac.service.in"
    );
    assert!(activation.contains("@RMAC_NOTIFICATION_EXEC@"));
    assert!(activation.contains("SystemdService=rmac-notification-center.service"));

    let center_activation = include_str!(
        "../../rmac-notifications-linux/install/org.rmac.NotificationCenter1.service.in"
    );
    assert!(center_activation.contains("Name=org.rmac.NotificationCenter1"));
    assert!(center_activation.contains("@RMAC_NOTIFICATION_EXEC@"));
    assert!(center_activation.contains("SystemdService=rmac-notification-center.service"));
}

#[test]
fn file_chooser_backend_is_a_separate_activated_portal() {
    let descriptor = include_str!("../../rmac-file-chooser/install/rmac-file-chooser.portal");
    assert!(descriptor.contains("DBusName=org.freedesktop.impl.portal.desktop.rmac.filechooser"));
    assert!(descriptor.contains("Interfaces=org.freedesktop.impl.portal.FileChooser;"));
    assert!(descriptor.contains("UseIn=rmac"));

    let activation = include_str!(
        "../../rmac-file-chooser/install/org.freedesktop.impl.portal.desktop.rmac.filechooser.service.in"
    );
    assert!(activation.contains("Name=org.freedesktop.impl.portal.desktop.rmac.filechooser"));
    assert!(activation.contains("@RMAC_FILE_CHOOSER_EXEC@"));
    assert!(activation.contains("SystemdService=rmac-file-chooser.service"));

    let unit = include_str!("../units/rmac-file-chooser.service");
    assert!(unit.contains("Type=dbus"));
    assert!(unit.contains("BusName=org.freedesktop.impl.portal.desktop.rmac.filechooser"));
    assert!(unit.contains("ExecStart=%h/.local/libexec/rmac/rmac-file-chooser"));
    assert!(unit.contains("NoNewPrivileges=yes"));
    assert!(!unit.contains("PrivateTmp"));
    assert!(!unit.contains("/bin/sh"));
    // Activated on demand: neither wanted by the session nor supervised.
    let target = include_str!("../units/rmac-session.target");
    assert!(!target.contains("rmac-file-chooser.service"));
    assert!(!COMPONENT_UNITS.contains(&"rmac-file-chooser.service"));
}

#[test]
fn focus_activation_asset_routes_to_the_supervised_authority() {
    let activation = include_str!("../../rmac-focus-linux/install/org.rmac.Focus1.service.in");
    assert!(activation.contains("Name=org.rmac.Focus1"));
    assert!(activation.contains("@RMAC_FOCUS_EXEC@"));
    assert!(activation.contains("SystemdService=rmac-focus.service"));
}
