//! Stable display service model.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    /// Refresh rate in millihertz.
    pub refresh_rate: u32,
    pub preferred: bool,
}

impl Mode {
    pub fn label(self) -> String {
        if self.refresh_rate == 0 {
            format!("{} × {}", self.width, self.height)
        } else {
            format!(
                "{} × {} at {:.3} Hz",
                self.width,
                self.height,
                f64::from(self.refresh_rate) / 1000.0
            )
        }
    }

    pub(super) fn niri_argument(self) -> String {
        format!(
            "{}x{}@{:.3}",
            self.width,
            self.height,
            f64::from(self.refresh_rate) / 1000.0
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
    Other(String),
}

impl Transform {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "Standard",
            Self::Rotate90 => "90°",
            Self::Rotate180 => "180°",
            Self::Rotate270 => "270°",
            Self::Flipped => "Flipped",
            Self::Flipped90 => "Flipped 90°",
            Self::Flipped180 => "Flipped 180°",
            Self::Flipped270 => "Flipped 270°",
            Self::Other(_) => "Unknown",
        }
    }

    pub fn is_configurable(&self) -> bool {
        !matches!(self, Self::Other(_))
    }

    pub(super) fn niri_argument(&self) -> Option<&'static str> {
        match self {
            Self::Normal => Some("normal"),
            Self::Rotate90 => Some("90"),
            Self::Rotate180 => Some("180"),
            Self::Rotate270 => Some("270"),
            Self::Flipped => Some("flipped"),
            Self::Flipped90 => Some("flipped-90"),
            Self::Flipped180 => Some("flipped-180"),
            Self::Flipped270 => Some("flipped-270"),
            Self::Other(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogicalOutput {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputConfiguration {
    pub id: String,
    pub mode: Mode,
    pub scale: f64,
    pub transform: Transform,
    pub x: i32,
    pub y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub outputs: Vec<OutputConfiguration>,
    pub primary: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Stable output name used to address this output through niri.
    pub id: String,
    pub connector: String,
    pub name: String,
    pub serial: Option<String>,
    pub physical_size_mm: Option<(u32, u32)>,
    pub modes: Vec<Mode>,
    pub current_mode: Option<usize>,
    pub logical: Option<LogicalOutput>,
    pub primary: bool,
    pub detail: Option<String>,
}

impl Output {
    pub fn current_mode(&self) -> Option<Mode> {
        self.current_mode
            .and_then(|index| self.modes.get(index).copied())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub can_configure: bool,
    pub can_persist: bool,
    pub mirror_supported: bool,
    pub compositor: String,
    pub graphics: Option<String>,
    pub outputs: Vec<Output>,
    pub persistence_detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedChange {
    pub baseline: Snapshot,
    pub snapshot: Snapshot,
}

#[derive(Debug)]
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
