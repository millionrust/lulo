//! Stable Dock runtime health, snapshot, update, and coordinator state.

use super::*;

#[derive(Clone, Default, Eq, PartialEq)]
pub enum SourceHealth {
    #[default]
    Starting,
    Healthy,
    Unavailable {
        detail: String,
    },
}

impl fmt::Debug for SourceHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => formatter.write_str("Starting"),
            Self::Healthy => formatter.write_str("Healthy"),
            Self::Unavailable { .. } => formatter
                .debug_struct("Unavailable")
                .field("detail", &"<redacted>")
                .finish(),
        }
    }
}

impl SourceHealth {
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unavailable { detail } => Some(detail),
            Self::Starting | Self::Healthy => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub compositor: SourceHealth,
    pub settings: SourceHealth,
    pub catalog: SourceHealth,
    pub displays: SourceHealth,
    pub places: SourceHealth,
    pub appearance: SourceHealth,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Complete compositor authority retained for renderer policies such as
    /// fullscreen suppression. Hosts must not rebuild Dock state from it.
    pub compositor: rmac_compositor::Snapshot,
    /// Effective Dock settings used to produce this exact snapshot.
    pub settings: rmac_shell_settings::DockSettings,
    pub model: rmac_dock::Model,
    /// Renderer-facing item groups, icons, indicators, badges, and semantics.
    pub content: rmac_dock::presentation::ShelfContent,
    pub outputs: Vec<rmac_compositor::OutputId>,
    /// Complete renderer-facing policy for every selected valid output.
    pub surface_plan: Result<Vec<rmac_dock::SurfaceDescription>, rmac_dock::motion::ConfigError>,
    /// Authoritative niri overview state consumed by each output's D6
    /// visibility machine. This is not inferred from focus or window geometry.
    pub overview_visible: bool,
    /// Effective rmac preference after resolving the host portal value through
    /// the writable theme authority.
    pub reduced_motion: bool,
    pub health: HealthSnapshot,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            compositor: rmac_compositor::Snapshot::default(),
            settings: rmac_shell_settings::DockSettings::default(),
            model: rmac_dock::Model::default(),
            content: rmac_dock::presentation::ShelfContent::default(),
            outputs: Vec::new(),
            surface_plan: Ok(Vec::new()),
            overview_visible: false,
            reduced_motion: false,
            health: HealthSnapshot::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(super) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
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

#[derive(Default)]
pub struct Coordinator {
    pub(super) compositor: rmac_compositor::State,
    pub(super) settings: rmac_shell_settings::ShellSettings,
    pub(super) catalog: Vec<rmac_apps::Application>,
    /// The "superseded in Lulo OS" desktop-ID list, refreshed alongside
    /// `catalog`. Only the Dock's quit-app suggestion strip consults it
    /// (`Coordinator::snapshot`); pinned and running items always resolve
    /// against the full `catalog` above.
    pub(super) superseded: std::collections::HashMap<String, String>,
    pub(super) places: Option<rmac_places::Snapshot>,
    pub(super) primary_output: Option<rmac_compositor::OutputId>,
    pub(super) reduced_motion: bool,
    pub(super) health: HealthSnapshot,
    /// The Dock's recent-apps section order (persisted by the consumer).
    pub(super) recents: Vec<String>,
    pub(super) recents_dirty: bool,
}
