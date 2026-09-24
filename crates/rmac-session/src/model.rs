use std::fmt;

use rmac_shell_settings::RecoveryState;
use rmac_storage::Failure;
use serde::{Deserialize, Serialize};

pub const COMPONENT_UNITS: [&str; 14] = [
    "rmac-top-bar.service",
    "rmac-dock.service",
    "rmac-launcher.service",
    "rmac-quick-settings.service",
    "rmac-notification-center.service",
    "rmac-notification-center-panel.service",
    "rmac-focus.service",
    "rmac-wallpaper.service",
    "rmac-osd.service",
    "rmac-app-switcher.service",
    "rmac-screenshot.service",
    "rmac-mission-control.service",
    "rmac-clipboard.service",
    "rmac-shortcut-broker.service",
];
/// Components without which the desktop cannot be operated. Only these may
/// stop the session for safe mode; any other component that exhausts its
/// restart budget stays down on its own while the rest of the desktop runs.
pub const ESSENTIAL_UNITS: [&str; 3] = [
    "rmac-top-bar.service",
    "rmac-dock.service",
    "rmac-shortcut-broker.service",
];
pub const RESTARTS_BEFORE_SAFE_MODE: u32 = 3;
pub(crate) const SAFE_MODE_VERSION: u32 = 1;
pub(crate) const SAFE_MODE_RECORD_VERSION: u32 = 1;

/// The executable each component unit starts, relative to the rmac libexec
/// directory that also holds the supervisor (package and development layouts
/// alike). A unit test holds this table to the unit files.
pub fn component_executable(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "rmac-top-bar.service" => "rmac-top-bar",
        "rmac-dock.service" => "rmac-dock",
        "rmac-launcher.service" => "rmac-launcher",
        "rmac-quick-settings.service" => "rmac-quick-settings",
        "rmac-notification-center.service" => "rmac-notification-center",
        "rmac-notification-center-panel.service" => "rmac-notification-center-panel",
        "rmac-focus.service" => "rmac-focus-service",
        "rmac-wallpaper.service" => "rmac-wallpaper",
        "rmac-osd.service" => "rmac-osd",
        "rmac-app-switcher.service" => "rmac-app-switcher",
        "rmac-screenshot.service" => "rmac-screenshot",
        "rmac-mission-control.service" => "rmac-mission-control",
        "rmac-clipboard.service" => "rmac-clipboard-service",
        "rmac-shortcut-broker.service" => "rmac-shortcut-broker",
        _ => return None,
    })
}

/// Names a component the way the safe-mode notice speaks about it.
pub fn component_display_name(unit: &str) -> &str {
    match unit {
        "rmac-top-bar.service" => "The menu bar",
        "rmac-dock.service" => "The Dock",
        "rmac-shortcut-broker.service" => "The keyboard shortcut service",
        "rmac-launcher.service" => "Spotlight",
        "rmac-notification-center.service" => "Notification Center",
        "rmac-wallpaper.service" => "The desktop picture",
        "rmac-osd.service" => "The volume and brightness display",
        other => other,
    }
}

/// Identifies one build of a component executable. A package or development
/// install replaces the file (new inode, size or mtime), so a safe-mode marker
/// recorded against an older build is known to be stale.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutableIdentity {
    pub path: String,
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_unix_s: i64,
    pub modified_nsec: i64,
}

/// What a login did with a persistent safe-mode marker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafeModeOutcome {
    /// This login runs in safe mode; the next one starts normally.
    SafeLogin,
    /// The failing component was rebuilt or reinstalled since it failed.
    SkippedChangedExecutable,
    /// The marker could not be read, so it cannot justify safe mode.
    DiscardedInvalidMarker,
}

/// `safe-mode.last.json`, and the runtime record of the current safe login.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SafeModeRecord {
    pub version: u32,
    pub consumed_at_unix_ms: u64,
    pub outcome: SafeModeOutcome,
    pub state: Option<SafeModeState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginMode {
    Normal,
    Safe,
}

impl LoginMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Safe => "safe",
        }
    }
}

/// Safe mode lasts one login, and is skipped outright when the executable
/// that exhausted its restart budget has since been replaced: that failure
/// belongs to a build that no longer exists. Markers written before builds
/// were recorded are honoured once.
pub fn login_outcome(
    state: &SafeModeState,
    current: Option<&ExecutableIdentity>,
) -> SafeModeOutcome {
    match &state.trigger_executable {
        None => SafeModeOutcome::SafeLogin,
        Some(recorded) if current == Some(recorded) => SafeModeOutcome::SafeLogin,
        Some(_) => SafeModeOutcome::SkippedChangedExecutable,
    }
}

/// When the notice is shown relative to the marker's lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeContext {
    /// This login consumed the marker; the next login is normal.
    ThisLogin,
    /// A component failed during this login; the marker is still pending.
    DuringSession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafeModeNotice {
    pub summary: String,
    pub body: String,
    pub action_label: &'static str,
}

pub const RESTART_NORMALLY_ACTION: &str = "restart-normally";

pub fn safe_mode_notice(trigger_unit: Option<&str>, context: NoticeContext) -> SafeModeNotice {
    let component = trigger_unit
        .map(component_display_name)
        .unwrap_or("An rmac component");
    let body = match context {
        NoticeContext::ThisLogin => format!(
            "{component} quit unexpectedly several times, so this login started in safe mode \
             without the menu bar, Dock and other rmac surfaces. Your next login will start \
             normally."
        ),
        NoticeContext::DuringSession => format!(
            "{component} quit unexpectedly several times, so rmac switched to safe mode. Your \
             next login will also start in safe mode unless you restart normally now."
        ),
    };
    SafeModeNotice {
        summary: "rmac is in Safe Mode".into(),
        body,
        action_label: "Restart Normally",
    }
}

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
    /// The failing unit's executable when its budget ran out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_executable: Option<ExecutableIdentity>,
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
    ConsumeSafeMode,
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
            Self::ConsumeSafeMode => "consume safe-mode state for this login",
            Self::ClearSafeMode => "clear safe-mode state",
            Self::WriteHealth => "write session health",
        })
    }
}

pub type Error = Failure<Operation>;
