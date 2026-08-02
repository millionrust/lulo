use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub enum SourceHealth {
    Starting,
    Healthy,
    Unavailable { detail: String },
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub compositor: SourceHealth,
    pub settings: SourceHealth,
    pub files: SourceHealth,
}

impl Default for HealthSnapshot {
    fn default() -> Self {
        Self {
            compositor: SourceHealth::Starting,
            settings: SourceHealth::Starting,
            files: SourceHealth::Healthy,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub plan: rmac_wallpaper::Plan,
    pub health: HealthSnapshot,
}

pub enum Update {
    Render {
        plan: rmac_wallpaper::Plan,
        rasterized: rmac_wallpaper_image::Rasterized,
        health: HealthSnapshot,
    },
    Health(HealthSnapshot),
}

impl fmt::Debug for Update {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render {
                plan,
                rasterized,
                health,
            } => formatter
                .debug_struct("Render")
                .field("outputs", &plan.surfaces.len())
                .field("plan_issues", &plan.issues)
                .field("raster_issues", &rasterized.issues)
                .field("health", health)
                .finish(),
            Self::Health(health) => formatter.debug_tuple("Health").field(health).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchCompositor,
    WatchSettings,
    Consume,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    detail: String,
}

impl Error {
    pub(crate) fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("operation", &self.operation)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not run the wallpaper service")
    }
}

impl std::error::Error for Error {}
