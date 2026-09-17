use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ActivationId, OutputId, WindowId, WorkspaceId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    Spawn,
    FocusWindow,
    FocusWorkspace,
    FocusOutput,
    CloseWindow,
    MoveWindowToWorkspace,
    MoveWindowToOutput,
    SetOverview,
    FullscreenWindow,
    FillWindow,
    CenterWindow,
    TileWindow,
    MinimizeWindow,
    RestoreWindow,
}

/// Target region for [`Action::TileWindow`]. niri's scrolling layout only
/// supports the horizontal `Left`/`Right` placements; the rest are rejected.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TileRegion {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The hidden workspace that holds minimized windows.
pub const PARKING_WORKSPACE: &str = "rmac-parking";

pub(crate) const MAX_SPAWN_ARGUMENTS: usize = 256;
const MAX_SPAWN_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_SPAWN_COMMAND_BYTES: usize = 128 * 1024;

/// Shell-free argv accepted by the compositor launch boundary. Debug output is
/// intentionally redacted because desktop-entry arguments can contain private
/// paths or application-defined values.
#[derive(Clone, Eq, PartialEq)]
pub struct SpawnCommand(Vec<String>);

impl SpawnCommand {
    pub fn new(arguments: Vec<String>) -> Result<Self, SpawnCommandError> {
        if arguments.is_empty() {
            return Err(SpawnCommandError::Empty);
        }
        if arguments[0].is_empty() {
            return Err(SpawnCommandError::EmptyProgram);
        }
        if arguments.len() > MAX_SPAWN_ARGUMENTS {
            return Err(SpawnCommandError::TooManyArguments);
        }
        let mut total = 0usize;
        for argument in &arguments {
            if argument.contains('\0') {
                return Err(SpawnCommandError::InteriorNul);
            }
            if argument.len() > MAX_SPAWN_ARGUMENT_BYTES {
                return Err(SpawnCommandError::ArgumentTooLong);
            }
            total = total.saturating_add(argument.len());
            if total > MAX_SPAWN_COMMAND_BYTES {
                return Err(SpawnCommandError::CommandTooLong);
            }
        }
        Ok(Self(arguments))
    }

    pub fn arguments(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Debug for SpawnCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpawnCommand")
            .field("argument_count", &self.0.len())
            .field(
                "total_bytes",
                &self.0.iter().map(String::len).sum::<usize>(),
            )
            .finish()
    }
}

impl Serialize for SpawnCommand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SpawnCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let arguments = Vec::<String>::deserialize(deserializer)?;
        Self::new(arguments).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnCommandError {
    Empty,
    EmptyProgram,
    TooManyArguments,
    ArgumentTooLong,
    CommandTooLong,
    InteriorNul,
}

impl fmt::Display for SpawnCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "spawn command is empty",
            Self::EmptyProgram => "spawn program is empty",
            Self::TooManyArguments => "spawn command has too many arguments",
            Self::ArgumentTooLong => "spawn argument is too long",
            Self::CommandTooLong => "spawn command is too long",
            Self::InteriorNul => "spawn argument contains an invalid byte",
        })
    }
}

impl std::error::Error for SpawnCommandError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    Spawn {
        command: SpawnCommand,
    },
    FocusWindow {
        window: WindowId,
    },
    FocusWorkspace {
        workspace: WorkspaceId,
    },
    FocusOutput {
        output: OutputId,
    },
    CloseWindow {
        window: WindowId,
    },
    MoveWindowToWorkspace {
        window: WindowId,
        workspace: WorkspaceId,
        follow: bool,
    },
    MoveWindowToOutput {
        window: WindowId,
        output: OutputId,
    },
    SetOverview {
        visible: bool,
    },
    /// Toggle full screen for `window`. niri exposes only a toggle, so a
    /// caller must send this only when the desired state differs from the
    /// current one; `on` records the requested target.
    FullscreenWindow {
        window: WindowId,
        on: bool,
    },
    /// Maximize `window` into the available width without full screen.
    FillWindow {
        window: WindowId,
    },
    /// Center `window` in its output.
    CenterWindow {
        window: WindowId,
    },
    /// Tile `window` into a screen region.
    TileWindow {
        window: WindowId,
        region: TileRegion,
    },
    /// Move `window` to the hidden parking workspace.
    MinimizeWindow {
        window: WindowId,
    },
    /// Move `window` back to `workspace` and focus it.
    RestoreWindow {
        window: WindowId,
        workspace: WorkspaceId,
    },
}

impl Action {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::Spawn { .. } => ActionKind::Spawn,
            Self::FocusWindow { .. } => ActionKind::FocusWindow,
            Self::FocusWorkspace { .. } => ActionKind::FocusWorkspace,
            Self::FocusOutput { .. } => ActionKind::FocusOutput,
            Self::CloseWindow { .. } => ActionKind::CloseWindow,
            Self::MoveWindowToWorkspace { .. } => ActionKind::MoveWindowToWorkspace,
            Self::MoveWindowToOutput { .. } => ActionKind::MoveWindowToOutput,
            Self::SetOverview { .. } => ActionKind::SetOverview,
            Self::FullscreenWindow { .. } => ActionKind::FullscreenWindow,
            Self::FillWindow { .. } => ActionKind::FillWindow,
            Self::CenterWindow { .. } => ActionKind::CenterWindow,
            Self::TileWindow { .. } => ActionKind::TileWindow,
            Self::MinimizeWindow { .. } => ActionKind::MinimizeWindow,
            Self::RestoreWindow { .. } => ActionKind::RestoreWindow,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionCapabilities {
    pub supported: Vec<ActionKind>,
}

impl ActionCapabilities {
    pub fn supports(&self, kind: ActionKind) -> bool {
        self.supported.contains(&kind)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionRequest {
    pub id: ActivationId,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionErrorKind {
    Unavailable,
    Transport,
    Protocol,
    Rejected,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionError {
    pub kind: ActionErrorKind,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionResult {
    pub id: ActivationId,
    pub action: Action,
    pub result: Result<(), ActionError>,
}
