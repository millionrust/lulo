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
