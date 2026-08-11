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
        health: root.join("runtime/health.json"),
    };
    (root, paths)
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
    assert!(!lock.contains("Restart="));
    assert!(!lock.contains("RestartSec="));
    assert!(lock.contains("StartLimitIntervalSec=30s"));
    assert!(lock.contains("StartLimitBurst=4"));
    assert!(lock.contains("OnFailure=rmac-lock-fallback.service"));
    assert!(lock.contains("ExecStart=%h/.local/libexec/rmac/rmac-lock-provider"));
    assert!(lock.contains("WatchdogSec=10s"));
    assert!(lock.contains("KillMode=control-group"));
    assert!(!lock.contains("OnFailure=rmac-component-failure"));
    assert!(!lock.contains("NoNewPrivileges=yes"));
    assert!(!lock.contains("/bin/sh"));

    let fallback = include_str!("../units/rmac-lock-fallback.service");
    assert!(fallback.contains("Type=notify"));
    assert!(fallback.contains("NotifyAccess=all"));
    assert!(fallback.contains("StartLimitIntervalSec=0"));
    assert!(fallback.contains("Restart=on-failure"));
    assert!(fallback.contains(
        "ExecStart=%h/.local/libexec/rmac/rmac-locker --config %h/.config/rmac/swaylock.conf"
    ));
    assert!(fallback.contains("KillMode=control-group"));
    assert!(!fallback.contains("/bin/sh"));

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
    assert!(!normal_target.contains("rmac-app-drawer.service"));
    assert!(normal_target.contains("rmac-osd.service"));
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

    let default_policy = include_str!("../lock-policy.json");
    assert!(default_policy.contains("\"version\": 1"));
    assert!(default_policy.contains("\"lock_after_seconds\": 300"));
    assert!(default_policy.contains("\"suspend_after_seconds\": null"));
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
    assert!(!selection.contains("org.freedesktop.impl.portal.FileChooser=rmac"));

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
fn focus_activation_asset_routes_to_the_supervised_authority() {
    let activation = include_str!("../../rmac-focus-linux/install/org.rmac.Focus1.service.in");
    assert!(activation.contains("Name=org.rmac.Focus1"));
    assert!(activation.contains("@RMAC_FOCUS_EXEC@"));
    assert!(activation.contains("SystemdService=rmac-focus.service"));
}
