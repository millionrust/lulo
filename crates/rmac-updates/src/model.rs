//! Stable update snapshot, source, and event model.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateKind {
    Security,
    Critical,
    Important,
    BugFix,
    Enhancement,
    Blocked,
    Low,
    #[default]
    Normal,
    Unknown,
}

impl UpdateKind {
    pub fn from_packagekit(value: u32) -> Self {
        match value {
            3 => Self::Low,
            4 => Self::Enhancement,
            5 => Self::Normal,
            6 => Self::BugFix,
            7 => Self::Important,
            8 => Self::Security,
            9 => Self::Blocked,
            26 => Self::Critical,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::Critical => "Critical",
            Self::Important => "Important",
            Self::BugFix => "Bug fix",
            Self::Enhancement => "Enhancement",
            Self::Blocked => "Blocked",
            Self::Low => "Low priority",
            Self::Normal | Self::Unknown => "Update",
        }
    }

    pub fn is_security_relevant(self) -> bool {
        matches!(self, Self::Security | Self::Critical)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub package_id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub kind: UpdateKind,
}

impl Update {
    pub fn from_packagekit(info: u32, package_id: &str, summary: &str) -> Option<Self> {
        let (name, version) = package_identity(package_id)?;
        Some(Self {
            package_id: package_id.to_string(),
            name,
            version,
            summary: bounded_text(summary),
            kind: UpdateKind::from_packagekit(info),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub updates: Vec<Update>,
    pub truncated: bool,
    pub install_supported: bool,
    pub install_unavailable_reason: Option<String>,
}

impl Snapshot {
    pub fn security_count(&self) -> usize {
        self.updates
            .iter()
            .filter(|update| update.kind.is_security_relevant())
            .count()
    }

    pub fn blocked_count(&self) -> usize {
        self.updates
            .iter()
            .filter(|update| update.kind == UpdateKind::Blocked)
            .count()
    }

    pub fn installable_updates(&self) -> impl Iterator<Item = &Update> {
        self.updates
            .iter()
            .filter(|update| update.kind != UpdateKind::Blocked)
    }

    pub fn installable_count(&self) -> usize {
        self.installable_updates().count()
    }

    pub fn installable_ids(&self) -> Vec<String> {
        let mut ids = self
            .installable_updates()
            .map(|update| update.package_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn can_prepare_install(&self) -> bool {
        self.install_supported && !self.truncated && self.installable_count() > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub cache_age_seconds: u32,
}

impl Request {
    pub const fn cached() -> Self {
        Self {
            cache_age_seconds: 3600,
        }
    }

    pub const fn refresh() -> Self {
        Self {
            cache_age_seconds: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Unavailable,
    Timeout,
    Protocol,
    Backend,
    Cancelled,
    Authorization,
    Trust,
    Interaction,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl AsRef<str>) -> Self {
        Self {
            kind,
            detail: bounded_text(detail.as_ref()),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub type SnapshotFuture<'a> = Pin<Box<dyn Future<Output = Result<Snapshot, Error>> + Send + 'a>>;

/// Injectable asynchronous boundary for PackageKit and deterministic fixtures.
pub trait Source {
    fn snapshot(&self, request: Request) -> SnapshotFuture<'_>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Package {
        info: u32,
        package_id: String,
        summary: String,
    },
    BackendError {
        code: u32,
        detail: String,
    },
    RestartRequired {
        kind: u32,
        package_id: String,
    },
    Finished {
        exit: u32,
    },
}
