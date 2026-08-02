use std::fmt;
use std::path::PathBuf;

use crate::entry::bounded_text;

pub const MAX_ITEMS: usize = 512;
pub const MAX_BACKGROUND_SERVICES: usize = 512;
pub const MAX_ISSUES: usize = 128;
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
pub(crate) const MAX_ERROR_BYTES: usize = 512;
pub(crate) const MAX_DISPLAY_FIELD_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    pub enabled: bool,
    pub applies_to_session: bool,
    pub session_detail: Option<String>,
    pub user_owned: bool,
    pub managed_override: bool,
    pub can_toggle: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub file: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddPreview {
    pub source: PathBuf,
    pub id: String,
    pub name: String,
    pub command: String,
    pub replacing: bool,
    source_contents: Vec<u8>,
    target_contents: Option<Vec<u8>>,
}

impl AddPreview {
    pub fn new(
        source: PathBuf,
        id: String,
        name: String,
        command: String,
        source_contents: Vec<u8>,
        target_contents: Option<Vec<u8>>,
    ) -> Self {
        Self {
            source,
            id,
            name,
            command,
            replacing: target_contents.is_some(),
            source_contents,
            target_contents,
        }
    }

    pub fn source_contents(&self) -> &[u8] {
        &self.source_contents
    }

    pub fn target_contents(&self) -> Option<&[u8]> {
        self.target_contents.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovePreview {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    contents: Vec<u8>,
}

impl RemovePreview {
    pub fn new(id: String, name: String, source: PathBuf, contents: Vec<u8>) -> Self {
        Self {
            id,
            name,
            source,
            contents,
        }
    }

    pub fn contents(&self) -> &[u8] {
        &self.contents
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitFileState {
    Enabled,
    Disabled,
    Linked,
    Runtime,
    Masked,
    Static,
    Other,
}

impl UnitFileState {
    pub fn from_systemd(value: &str) -> Self {
        match value {
            "enabled" => Self::Enabled,
            "disabled" => Self::Disabled,
            "linked" => Self::Linked,
            "enabled-runtime" | "linked-runtime" => Self::Runtime,
            "masked" | "masked-runtime" => Self::Masked,
            "static" | "indirect" | "generated" | "transient" => Self::Static,
            _ => Self::Other,
        }
    }

    pub fn enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::Linked | Self::Runtime)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Disabled => "Disabled",
            Self::Linked => "Linked",
            Self::Runtime => "Runtime only",
            Self::Masked => "Masked",
            Self::Static => "Static",
            Self::Other => "Unmanaged state",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundService {
    pub id: String,
    pub name: String,
    pub state: UnitFileState,
    pub enabled: bool,
    pub can_toggle: bool,
    pub detail: String,
    pub user_owned: bool,
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub background_services: Vec<BackgroundService>,
    pub background_services_truncated: bool,
    pub background_services_error: Option<String>,
    pub issues: Vec<Issue>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedEntry {
    pub name: String,
    pub command: String,
    pub hidden: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub try_exec: Option<String>,
    pub managed_override: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidEntry,
    Unavailable,
    Conflict,
    Mutation,
    Mismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind,
            detail: bounded_text(&detail),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error>;
    fn set_background_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error>;
    fn prepare_add(&self, source: &std::path::Path) -> Result<AddPreview, Error>;
    fn add(&self, preview: &AddPreview) -> Result<Snapshot, Error>;
    fn prepare_remove(&self, id: &str) -> Result<RemovePreview, Error>;
    fn remove(&self, preview: &RemovePreview) -> Result<Snapshot, Error>;
}
