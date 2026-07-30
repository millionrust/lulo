//! Typed health and safe-mode state for the systemd-supervised rmac session.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_shell_settings::RecoveryState;
use rmac_storage::{atomic_write, Failure};
use serde::{Deserialize, Serialize};

pub const COMPONENT_UNITS: [&str; 10] = [
    "rmac-top-bar.service",
    "rmac-dock.service",
    "rmac-launcher.service",
    "rmac-app-drawer.service",
    "rmac-quick-settings.service",
    "rmac-notification-center.service",
    "rmac-notification-center-panel.service",
    "rmac-focus.service",
    "rmac-wallpaper.service",
    "rmac-shortcut-broker.service",
];
pub const RESTARTS_BEFORE_SAFE_MODE: u32 = 3;
const SAFE_MODE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentHealth {
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub result: String,
    pub restarts: u32,
    pub main_pid: Option<u32>,
    pub exit_status: Option<i32>,
}

impl ComponentHealth {
    pub fn available(&self) -> bool {
        self.load_state == "loaded"
    }

    pub fn healthy(&self) -> bool {
        self.available() && self.active_state == "active" && self.sub_state == "running"
    }

    pub fn exhausted_restart_budget(&self) -> bool {
        self.result == "start-limit-hit" || self.restarts >= RESTARTS_BEFORE_SAFE_MODE
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SafeModeState {
    pub version: u32,
    pub entered_at_unix_ms: u64,
    pub trigger_unit: String,
    pub observed_restarts: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionHealth {
    pub observed_at_unix_ms: u64,
    pub safe_mode: Option<SafeModeState>,
    pub components: Vec<ComponentHealth>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticComponent {
    pub unit: String,
    pub available: bool,
    pub healthy: bool,
    pub restart_budget_exhausted: bool,
    pub restarts: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticReport {
    pub format: u32,
    pub observed_at_unix_ms: u64,
    pub safe_mode: bool,
    pub safe_mode_trigger_unit: Option<String>,
    pub shell_settings_recovery: RecoveryState,
    pub components: Vec<DiagnosticComponent>,
}

impl DiagnosticReport {
    pub fn from_health(health: SessionHealth, shell_settings_recovery: RecoveryState) -> Self {
        let safe_mode_trigger_unit = health
            .safe_mode
            .as_ref()
            .map(|state| state.trigger_unit.clone());
        Self {
            format: 1,
            observed_at_unix_ms: health.observed_at_unix_ms,
            safe_mode: health.safe_mode.is_some(),
            safe_mode_trigger_unit,
            shell_settings_recovery,
            components: health
                .components
                .into_iter()
                .map(|component| {
                    let available = component.available();
                    let healthy = component.healthy();
                    let restart_budget_exhausted = component.exhausted_restart_budget();
                    DiagnosticComponent {
                        unit: component.unit,
                        available,
                        healthy,
                        restart_budget_exhausted,
                        restarts: component.restarts,
                    }
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolvePath,
    RunSystemctl,
    ParseSystemctl,
    ReadSafeMode,
    WriteSafeMode,
    ClearSafeMode,
    WriteHealth,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolvePath => "resolve session state paths",
            Self::RunSystemctl => "query the systemd user manager",
            Self::ParseSystemctl => "parse systemd component health",
            Self::ReadSafeMode => "read safe-mode state",
            Self::WriteSafeMode => "write safe-mode state",
            Self::ClearSafeMode => "clear safe-mode state",
            Self::WriteHealth => "write session health",
        })
    }
}

pub type Error = Failure<Operation>;

pub trait CommandRunner {
    fn output(&self, program: &str, arguments: &[&str]) -> io::Result<Output>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn output(&self, program: &str, arguments: &[&str]) -> io::Result<Output> {
        Command::new(program).args(arguments).output()
    }
}

#[derive(Clone, Debug)]
pub struct StatePaths {
    pub safe_mode: PathBuf,
    pub health: PathBuf,
}

impl StatePaths {
    pub fn from_environment() -> Result<Self, Error> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Failure::message(
                Operation::ResolvePath,
                Path::new("session"),
                "HOME is not set",
            )
        })?;
        let state_root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".local/state"));
        let runtime_root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                Failure::message(
                    Operation::ResolvePath,
                    Path::new("session-health.json"),
                    "XDG_RUNTIME_DIR is not set to an absolute path",
                )
            })?;
        Ok(Self {
            safe_mode: state_root.join("rmac/session/safe-mode.json"),
            health: runtime_root.join("rmac/session-health.json"),
        })
    }
}

pub struct Supervisor<R = ProcessRunner> {
    paths: StatePaths,
    runner: R,
}

impl Supervisor<ProcessRunner> {
    pub fn from_environment() -> Result<Self, Error> {
        Ok(Self::new(StatePaths::from_environment()?, ProcessRunner))
    }
}

impl<R: CommandRunner> Supervisor<R> {
    pub fn new(paths: StatePaths, runner: R) -> Self {
        Self { paths, runner }
    }

    pub fn snapshot(&self) -> Result<SessionHealth, Error> {
        let mut components = Vec::with_capacity(COMPONENT_UNITS.len());
        for unit in COMPONENT_UNITS {
            components.push(self.component_health(unit)?);
        }
        Ok(SessionHealth {
            observed_at_unix_ms: now_unix_ms(),
            safe_mode: self.load_safe_mode()?,
            components,
        })
    }

    pub fn component_health(&self, unit: &str) -> Result<ComponentHealth, Error> {
        validate_component_unit(unit, &self.paths.health)?;
        let output = self
            .runner
            .output(
                "systemctl",
                &[
                    "--user",
                    "show",
                    unit,
                    "--no-pager",
                    "--property=Id,LoadState,ActiveState,SubState,Result,NRestarts,MainPID,ExecMainStatus",
                ],
            )
            .map_err(|error| Failure::from_io(Operation::RunSystemctl, &self.paths.health, error))?;
        if !output.status.success() && output.stdout.is_empty() {
            return Err(Failure::message_with_kind(
                Operation::RunSystemctl,
                &self.paths.health,
                io::ErrorKind::NotConnected,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        let text = String::from_utf8(output.stdout).map_err(|error| {
            Failure::message(
                Operation::ParseSystemctl,
                &self.paths.health,
                error.to_string(),
            )
        })?;
        parse_component_health(&text, unit).map_err(|detail| {
            Failure::message(Operation::ParseSystemctl, &self.paths.health, detail)
        })
    }

    pub fn write_health(&self) -> Result<SessionHealth, Error> {
        let snapshot = self.snapshot()?;
        write_json(&self.paths.health, &snapshot, Operation::WriteHealth)?;
        Ok(snapshot)
    }

    pub fn observe_failure(&self, unit: &str) -> Result<bool, Error> {
        let health = self.component_health(unit)?;
        if !health.exhausted_restart_budget() {
            self.write_health()?;
            return Ok(false);
        }
        let state = SafeModeState {
            version: SAFE_MODE_VERSION,
            entered_at_unix_ms: now_unix_ms(),
            trigger_unit: unit.to_owned(),
            observed_restarts: health.restarts,
            reason: if health.result == "start-limit-hit" {
                "systemd start limit was reached".into()
            } else {
                "component exhausted the bounded restart budget".into()
            },
        };
        write_json(&self.paths.safe_mode, &state, Operation::WriteSafeMode)?;
        Ok(true)
    }

    pub fn load_safe_mode(&self) -> Result<Option<SafeModeState>, Error> {
        let bytes = match std::fs::read(&self.paths.safe_mode) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Failure::from_io(
                    Operation::ReadSafeMode,
                    &self.paths.safe_mode,
                    error,
                ));
            }
        };
        let state: SafeModeState = serde_json::from_slice(&bytes).map_err(|error| {
            Failure::message(
                Operation::ReadSafeMode,
                &self.paths.safe_mode,
                error.to_string(),
            )
        })?;
        if state.version != SAFE_MODE_VERSION {
            return Err(Failure::message(
                Operation::ReadSafeMode,
                &self.paths.safe_mode,
                format!("unsupported safe-mode version {}", state.version),
            ));
        }
        validate_component_unit(&state.trigger_unit, &self.paths.safe_mode)?;
        Ok(Some(state))
    }

    pub fn clear_safe_mode(&self) -> Result<(), Error> {
        match std::fs::remove_file(&self.paths.safe_mode) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Failure::from_io(
                Operation::ClearSafeMode,
                &self.paths.safe_mode,
                error,
            )),
        }
    }
}

pub fn parse_component_health(text: &str, expected_unit: &str) -> Result<ComponentHealth, String> {
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed systemctl property: {line}"))?;
        fields.insert(key, value);
    }
    let unit = required(&fields, "Id")?;
    if unit != expected_unit {
        return Err(format!(
            "systemctl returned {unit} while querying {expected_unit}"
        ));
    }
    Ok(ComponentHealth {
        unit: unit.to_owned(),
        load_state: required(&fields, "LoadState")?.to_owned(),
        active_state: required(&fields, "ActiveState")?.to_owned(),
        sub_state: required(&fields, "SubState")?.to_owned(),
        result: required(&fields, "Result")?.to_owned(),
        restarts: parse_number(&fields, "NRestarts")?,
        main_pid: nonzero_number(&fields, "MainPID")?,
        exit_status: optional_number(&fields, "ExecMainStatus")?,
    })
}

fn required<'a>(
    fields: &'a std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<&'a str, String> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| format!("systemctl omitted {key}"))
}

fn parse_number<T: std::str::FromStr>(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<T, String> {
    required(fields, key)?
        .parse()
        .map_err(|_| format!("systemctl returned an invalid {key}"))
}

fn nonzero_number(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<Option<u32>, String> {
    Ok(match parse_number(fields, key)? {
        0 => None,
        value => Some(value),
    })
}

fn optional_number(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<Option<i32>, String> {
    let value = required(fields, key)?;
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| format!("systemctl returned an invalid {key}"))
    }
}

fn validate_component_unit(unit: &str, path: &Path) -> Result<(), Error> {
    if COMPONENT_UNITS.contains(&unit) {
        Ok(())
    } else {
        Err(Failure::message(
            Operation::ParseSystemctl,
            path,
            format!("unrecognized rmac component unit {unit}"),
        ))
    }
}

fn write_json(path: &Path, value: &impl Serialize, operation: Operation) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Failure::message(Operation::ResolvePath, path, "state path has no parent")
    })?;
    std::fs::create_dir_all(parent).map_err(|error| Failure::from_io(operation, parent, error))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| Failure::message(operation, path, error.to_string()))?;
    atomic_write(path, &bytes).map_err(|error| Failure::from_io(operation, path, error))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;
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
        let root =
            std::env::temp_dir().join(format!("rmac-session-{label}-{}", std::process::id()));
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
        let units = [
            include_str!("../units/rmac-top-bar.service"),
            include_str!("../units/rmac-dock.service"),
            include_str!("../units/rmac-launcher.service"),
            include_str!("../units/rmac-app-drawer.service"),
            include_str!("../units/rmac-quick-settings.service"),
            include_str!("../units/rmac-notification-center.service"),
            include_str!("../units/rmac-notification-center-panel.service"),
            include_str!("../units/rmac-focus.service"),
            include_str!("../units/rmac-wallpaper.service"),
            include_str!("../units/rmac-shortcut-broker.service"),
        ];
        for unit in units {
            assert!(unit.contains("Restart=on-failure"));
            assert!(unit.contains("RestartSec=1s"));
            assert!(unit.contains("StartLimitIntervalSec=60s"));
            assert!(unit.contains("StartLimitBurst=4"));
            assert!(unit.contains("OnFailure=rmac-component-failure@%N.service"));
            assert!(unit.contains("ConditionPathIsExecutable=%h/.local/libexec/rmac/"));
            assert!(!unit.contains("/bin/sh"));
        }
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
        assert!(shortcut_broker.contains("rmac-app-drawer.service"));
        assert!(shortcut_broker.contains("rmac-notification-center-panel.service"));

        let lock = include_str!("../units/rmac-lock.service");
        assert!(lock.contains("Type=notify"));
        assert!(lock.contains("NotifyAccess=all"));
        assert!(lock.contains("Restart=on-failure"));
        assert!(lock.contains("StartLimitIntervalSec=0"));
        assert!(lock.contains("KillMode=control-group"));
        assert!(!lock.contains("OnFailure=rmac-component-failure"));
        assert!(!lock.contains("NoNewPrivileges=yes"));
        assert!(!lock.contains("/bin/sh"));

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
}
