use std::fmt;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SourceHealth {
    #[default]
    Starting,
    Healthy,
    Unavailable {
        detail: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub compositor: SourceHealth,
    pub settings: SourceHealth,
    pub focus: SourceHealth,
    pub notifications: SourceHealth,
    pub network: SourceHealth,
    pub bluetooth: SourceHealth,
    pub audio: SourceHealth,
    pub power: SourceHealth,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub status: rmac_shell_status::Snapshot,
    pub quick_settings: rmac_quick_settings::Inputs,
    pub health: HealthSnapshot,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub snapshot: Snapshot,
    /// Whether the compact top bar should request a frame.
    pub visible: bool,
    /// Whether an open Quick Settings surface should request a frame.
    pub quick_settings_visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}
