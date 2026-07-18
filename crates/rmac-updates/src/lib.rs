//! Platform-neutral software-update snapshots, plans, and progress rules.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const MAX_UPDATES: usize = 512;
pub const MAX_PLAN_CHANGES: usize = 1024;
const MAX_PACKAGE_ID_BYTES: usize = 1024;
const MAX_TEXT_BYTES: usize = 512;

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

#[derive(Default)]
pub struct Collector {
    snapshot: Snapshot,
    backend_error: Option<Error>,
    finished: bool,
}

impl Collector {
    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service sent data after finishing",
            ));
        }
        match event {
            Event::Package {
                info,
                package_id,
                summary,
            } => {
                let Some(update) = Update::from_packagekit(info, &package_id, &summary) else {
                    return Err(Error::new(
                        ErrorKind::Protocol,
                        "the update service returned an invalid package identifier",
                    ));
                };
                if !self
                    .snapshot
                    .updates
                    .iter()
                    .any(|existing| existing.package_id == update.package_id)
                {
                    if self.snapshot.updates.len() >= MAX_UPDATES {
                        self.snapshot.truncated = true;
                    } else {
                        self.snapshot.updates.push(update);
                    }
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(packagekit_error(code, &detail));
            }
            Event::RestartRequired { .. } => {}
            Event::Finished { exit } => {
                self.finished = true;
                finish_exit(exit, self.backend_error.take(), "update check")?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<Snapshot, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service ended without a completion signal",
            ));
        }
        self.snapshot
            .updates
            .sort_by(|left, right| left.package_id.cmp(&right.package_id));
        Ok(self.snapshot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Update,
    Install,
    Remove,
    Obsolete,
    Reinstall,
    Downgrade,
}

impl ChangeKind {
    pub fn from_packagekit(info: u32) -> Result<Self, Error> {
        match info {
            11 => Ok(Self::Update),
            12 | 27 => Ok(Self::Install),
            13 | 28 => Ok(Self::Remove),
            15 | 29 => Ok(Self::Obsolete),
            19 => Ok(Self::Reinstall),
            20 | 30 => Ok(Self::Downgrade),
            23 => Err(Error::new(
                ErrorKind::Trust,
                "the update plan contains an untrusted package",
            )),
            _ => Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation returned an unknown package action",
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Update",
            Self::Install => "Install",
            Self::Remove => "Remove",
            Self::Obsolete => "Replace",
            Self::Reinstall => "Reinstall",
            Self::Downgrade => "Downgrade",
        }
    }

    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Remove | Self::Obsolete | Self::Downgrade)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedChange {
    pub package_id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub kind: ChangeKind,
}

impl PlannedChange {
    fn from_packagekit(info: u32, package_id: &str, summary: &str) -> Result<Self, Error> {
        let kind = ChangeKind::from_packagekit(info)?;
        let (name, version) = package_identity(package_id).ok_or_else(|| {
            Error::new(
                ErrorKind::Protocol,
                "the update simulation returned an invalid package identifier",
            )
        })?;
        Ok(Self {
            package_id: package_id.to_string(),
            name,
            version,
            summary: bounded_text(summary),
            kind,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub requested: Vec<Update>,
    pub changes: Vec<PlannedChange>,
    pub restart: RestartRequirement,
}

impl InstallPlan {
    pub fn requested_ids(&self) -> Vec<String> {
        self.requested
            .iter()
            .map(|update| update.package_id.clone())
            .collect()
    }

    pub fn has_destructive_changes(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.kind.is_destructive())
    }

    pub fn change_count(&self, kind: ChangeKind) -> usize {
        self.changes
            .iter()
            .filter(|change| change.kind == kind)
            .count()
    }
}

pub struct PlanCollector {
    requested: Vec<Update>,
    changes: Vec<PlannedChange>,
    restart: RestartRequirement,
    backend_error: Option<Error>,
    finished: bool,
}

impl PlanCollector {
    pub fn new(snapshot: &Snapshot) -> Result<Self, Error> {
        if !snapshot.install_supported {
            return Err(Error::new(
                ErrorKind::Unavailable,
                snapshot
                    .install_unavailable_reason
                    .as_deref()
                    .unwrap_or("the update backend cannot install updates"),
            ));
        }
        if snapshot.truncated {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the complete update set is too large to confirm safely",
            ));
        }
        let mut requested = snapshot.installable_updates().cloned().collect::<Vec<_>>();
        requested.sort_by(|left, right| left.package_id.cmp(&right.package_id));
        if requested.is_empty() {
            return Err(Error::new(
                ErrorKind::Stale,
                "no installable updates remain",
            ));
        }
        Ok(Self {
            requested,
            changes: Vec::new(),
            restart: RestartRequirement::None,
            backend_error: None,
            finished: false,
        })
    }

    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation sent data after finishing",
            ));
        }
        match event {
            Event::Package {
                info,
                package_id,
                summary,
            } => {
                let change = PlannedChange::from_packagekit(info, &package_id, &summary)?;
                if !self.changes.iter().any(|existing| {
                    existing.package_id == change.package_id && existing.kind == change.kind
                }) {
                    if self.changes.len() >= MAX_PLAN_CHANGES {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            "the update simulation is too large to confirm safely",
                        ));
                    }
                    self.changes.push(change);
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(packagekit_error(code, &detail));
            }
            Event::RestartRequired { kind, .. } => {
                self.restart = self.restart.max(RestartRequirement::from_packagekit(kind));
            }
            Event::Finished { exit } => {
                self.finished = true;
                finish_exit(exit, self.backend_error.take(), "update simulation")?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<InstallPlan, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation ended without a completion signal",
            ));
        }
        if self.changes.is_empty() {
            return Err(Error::new(
                ErrorKind::Stale,
                "the update simulation found no package changes",
            ));
        }
        self.changes.sort_by(|left, right| {
            left.package_id
                .cmp(&right.package_id)
                .then_with(|| change_rank(left.kind).cmp(&change_rank(right.kind)))
        });
        Ok(InstallPlan {
            requested: self.requested,
            changes: self.changes,
            restart: self.restart,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum RestartRequirement {
    #[default]
    None,
    Application,
    Session,
    System,
    SecuritySession,
    SecuritySystem,
    Unknown,
}

impl RestartRequirement {
    pub fn from_packagekit(value: u32) -> Self {
        match value {
            1 => Self::None,
            2 => Self::Application,
            3 => Self::Session,
            4 => Self::System,
            5 => Self::SecuritySession,
            6 => Self::SecuritySystem,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Application => Some("Restart affected applications"),
            Self::Session => Some("Sign out and back in"),
            Self::System => Some("Restart this computer"),
            Self::SecuritySession => Some("Sign out to finish a security update"),
            Self::SecuritySystem => Some("Restart to finish a security update"),
            Self::Unknown => Some("A restart may be required"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallPhase {
    Waiting,
    WaitingForLock,
    WaitingForAuthorization,
    Resolving,
    Downloading,
    Verifying,
    Installing,
    Updating,
    Removing,
    CleaningUp,
    Committing,
    Cancelling,
    Complete,
    #[default]
    Preparing,
}

impl InstallPhase {
    pub fn from_packagekit(value: u32) -> Self {
        match value {
            1 => Self::Waiting,
            6 => Self::Removing,
            8 | 20..=25 => Self::Downloading,
            9 => Self::Installing,
            10 => Self::Updating,
            11 => Self::CleaningUp,
            13 => Self::Resolving,
            14 => Self::Verifying,
            15 => Self::Preparing,
            16 => Self::Committing,
            18 => Self::Complete,
            19 => Self::Cancelling,
            30 => Self::WaitingForLock,
            31 => Self::WaitingForAuthorization,
            _ => Self::Preparing,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting to update…",
            Self::WaitingForLock => "Waiting for the package manager…",
            Self::WaitingForAuthorization => "Waiting for authorization…",
            Self::Resolving => "Resolving dependencies…",
            Self::Downloading => "Downloading updates…",
            Self::Verifying => "Verifying signatures…",
            Self::Installing => "Installing packages…",
            Self::Updating => "Installing updates…",
            Self::Removing => "Removing replaced packages…",
            Self::CleaningUp => "Cleaning up…",
            Self::Committing => "Committing package changes…",
            Self::Cancelling => "Cancelling safely…",
            Self::Complete => "Updates installed",
            Self::Preparing => "Preparing updates…",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallProgress {
    pub phase: InstallPhase,
    pub percentage: Option<u8>,
    pub allow_cancel: bool,
    pub remaining_seconds: Option<u32>,
    pub current_package: Option<String>,
    pub restart: RestartRequirement,
}

impl Default for InstallProgress {
    fn default() -> Self {
        Self {
            phase: InstallPhase::Preparing,
            percentage: None,
            allow_cancel: false,
            remaining_seconds: None,
            current_package: None,
            restart: RestartRequirement::None,
        }
    }
}

impl InstallProgress {
    pub fn set_percentage(&mut self, percentage: u32) {
        self.percentage = u8::try_from(percentage).ok().filter(|value| *value <= 100);
    }

    pub fn set_current_package(&mut self, package_id: &str) {
        self.current_package = package_identity(package_id).map(|(name, _)| name);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallResult {
    pub restart: RestartRequirement,
    pub changed_packages: usize,
}

#[derive(Default)]
pub struct InstallCollector {
    backend_error: Option<Error>,
    restart: RestartRequirement,
    changed_package_ids: std::collections::BTreeSet<String>,
    finished: bool,
}

impl InstallCollector {
    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update transaction sent data after finishing",
            ));
        }
        match event {
            Event::Package { package_id, .. } => {
                package_identity(&package_id).ok_or_else(|| {
                    Error::new(
                        ErrorKind::Protocol,
                        "the update transaction returned an invalid package identifier",
                    )
                })?;
                if !self.changed_package_ids.contains(&package_id) {
                    if self.changed_package_ids.len() >= MAX_PLAN_CHANGES {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            "the update transaction reported too many package changes",
                        ));
                    }
                    self.changed_package_ids.insert(package_id);
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(packagekit_error(code, &detail));
            }
            Event::RestartRequired { kind, .. } => {
                self.restart = self.restart.max(RestartRequirement::from_packagekit(kind));
            }
            Event::Finished { exit } => {
                self.finished = true;
                finish_exit(exit, self.backend_error.take(), "update installation")?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<InstallResult, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update transaction ended without a completion signal",
            ));
        }
        Ok(InstallResult {
            restart: self.restart,
            changed_packages: self.changed_package_ids.len(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

fn finish_exit(exit: u32, backend_error: Option<Error>, operation: &str) -> Result<(), Error> {
    match exit {
        1 => backend_error.map_or(Ok(()), Err),
        3 | 9 => Err(Error::new(
            ErrorKind::Cancelled,
            format!("the {operation} was cancelled"),
        )),
        4 => Err(Error::new(
            ErrorKind::Trust,
            "a repository signing key must be reviewed outside rmac",
        )),
        5 => Err(Error::new(
            ErrorKind::Interaction,
            "a package licence must be reviewed outside rmac",
        )),
        7 => Err(Error::new(
            ErrorKind::Interaction,
            "the package backend requires installation media",
        )),
        8 => Err(Error::new(
            ErrorKind::Trust,
            "the transaction requires an untrusted package",
        )),
        _ => Err(backend_error.unwrap_or_else(|| {
            Error::new(
                ErrorKind::Backend,
                format!("the update service finished with status {exit}"),
            )
        })),
    }
}

fn packagekit_error(code: u32, _detail: &str) -> Error {
    match code {
        17 | 65 => Error::new(ErrorKind::Cancelled, "the update transaction was cancelled"),
        3 => Error::new(
            ErrorKind::Unavailable,
            "the PackageKit backend does not support this update operation",
        ),
        2 => Error::new(
            ErrorKind::Backend,
            "a network connection is required to download the updates",
        ),
        10 | 37 | 43 | 64 => Error::new(
            ErrorKind::Backend,
            "PackageKit could not download update data from the configured repositories",
        ),
        13 => Error::new(
            ErrorKind::Backend,
            "the update dependencies could not be resolved",
        ),
        26 | 67 => Error::new(
            ErrorKind::Backend,
            "another package transaction currently holds the package-manager lock",
        ),
        5 | 30 | 31 | 50 | 51 => Error::new(
            ErrorKind::Trust,
            "PackageKit refused an untrusted or invalidly signed package",
        ),
        34 | 47 => Error::new(
            ErrorKind::Interaction,
            "PackageKit requires a licence or installation media that rmac cannot accept silently",
        ),
        46 => Error::new(
            ErrorKind::Backend,
            "there is not enough disk space to install the updates",
        ),
        48 => Error::new(
            ErrorKind::Authorization,
            "update authorization was denied or cancelled",
        ),
        27 | 41 | 49 => Error::new(ErrorKind::Stale, "no installable updates remain"),
        61 => Error::new(
            ErrorKind::Stale,
            "the package database changed during the update transaction",
        ),
        35 | 36 | 39 | 60 => Error::new(
            ErrorKind::Backend,
            "a package conflict prevents the update plan from being installed",
        ),
        38 | 40 | 56..=59 | 66 => Error::new(
            ErrorKind::Backend,
            "the package backend could not complete the update transaction safely",
        ),
        _ => Error::new(
            ErrorKind::Backend,
            format!("the package backend reported error {code}"),
        ),
    }
}

fn package_identity(package_id: &str) -> Option<(String, String)> {
    if package_id.is_empty()
        || package_id.len() > MAX_PACKAGE_ID_BYTES
        || package_id.chars().any(char::is_control)
    {
        return None;
    }
    let mut fields = package_id.split(';');
    let name = fields.next()?;
    let version = fields.next()?;
    let _architecture = fields.next()?;
    let _repository = fields.next()?;
    if fields.next().is_some() || name.is_empty() || version.is_empty() {
        return None;
    }
    let name = bounded_text(name);
    let version = bounded_text(version);
    (!name.is_empty() && !version.is_empty()).then_some((name, version))
}

fn change_rank(kind: ChangeKind) -> u8 {
    match kind {
        ChangeKind::Update => 0,
        ChangeKind::Install => 1,
        ChangeKind::Reinstall => 2,
        ChangeKind::Remove => 3,
        ChangeKind::Obsolete => 4,
        ChangeKind::Downgrade => 5,
    }
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_TEXT_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSource {
        result: Result<Snapshot, Error>,
    }

    impl Source for FakeSource {
        fn snapshot(&self, _request: Request) -> SnapshotFuture<'_> {
            let result = self.result.clone();
            Box::pin(async move { result })
        }
    }

    fn package(kind: u32, id: &str) -> Event {
        Event::Package {
            info: kind,
            package_id: id.into(),
            summary: "Package summary".into(),
        }
    }

    fn installable_snapshot() -> Snapshot {
        Snapshot {
            updates: vec![
                Update::from_packagekit(8, "kernel;6.18;amd64;updates", "Kernel update").unwrap(),
            ],
            install_supported: true,
            ..Snapshot::default()
        }
    }

    #[test]
    fn collector_classifies_and_deduplicates_modern_packagekit_enums() {
        let mut collector = Collector::default();
        collector
            .apply(package(8, "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package(8, "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package(9, "driver;2.0;amd64;updates"))
            .unwrap();
        collector.apply(Event::Finished { exit: 1 }).unwrap();

        let snapshot = collector.finish().unwrap();
        assert_eq!(snapshot.updates.len(), 2);
        assert_eq!(snapshot.security_count(), 1);
        assert_eq!(snapshot.blocked_count(), 1);
    }

    #[test]
    fn backend_failure_is_typed_without_exposing_backend_text() {
        let mut collector = Collector::default();
        collector
            .apply(Event::BackendError {
                code: 48,
                detail: "denied for /private/path\nuser name".into(),
            })
            .unwrap();
        let error = collector.apply(Event::Finished { exit: 2 }).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Authorization);
        assert!(!error.detail().contains("private"));
        assert!(!error.detail().contains("user name"));
    }

    #[test]
    fn common_backend_failures_have_private_safe_actionable_messages() {
        let network = packagekit_error(2, "/private/repository token");
        assert_eq!(network.kind(), ErrorKind::Backend);
        assert!(network.detail().contains("network"));
        assert!(!network.detail().contains("private"));

        let lock = packagekit_error(26, "pid 42 owned by user");
        assert!(lock.detail().contains("lock"));
        assert!(!lock.detail().contains("user"));

        let space = packagekit_error(46, "/var is full");
        assert!(space.detail().contains("disk space"));
        assert!(!space.detail().contains("/var"));
    }

    #[test]
    fn malformed_package_ids_fail_closed() {
        let mut collector = Collector::default();
        let error = collector.apply(package(5, "missing-version")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Protocol);
        assert!(Update::from_packagekit(5, "name;1;arch;repo;extra", "bad").is_none());
    }

    #[test]
    fn collector_bounds_untrusted_package_volume_and_ids() {
        let mut collector = Collector::default();
        for index in 0..=MAX_UPDATES {
            collector
                .apply(package(5, &format!("package-{index};1.0;amd64;updates")))
                .unwrap();
        }
        collector.apply(Event::Finished { exit: 1 }).unwrap();

        let snapshot = collector.finish().unwrap();
        assert_eq!(snapshot.updates.len(), MAX_UPDATES);
        assert!(snapshot.truncated);
        assert!(Update::from_packagekit(
            5,
            &format!("name;{};amd64;updates", "v".repeat(MAX_PACKAGE_ID_BYTES)),
            "oversized",
        )
        .is_none());
    }

    #[test]
    fn simulation_collects_exact_changes_and_restart_requirement() {
        let mut collector = PlanCollector::new(&installable_snapshot()).unwrap();
        collector
            .apply(package(11, "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package(12, "kernel-helper;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(Event::RestartRequired {
                kind: 6,
                package_id: "kernel;6.18;amd64;updates".into(),
            })
            .unwrap();
        collector.apply(Event::Finished { exit: 1 }).unwrap();

        let plan = collector.finish().unwrap();
        assert_eq!(plan.requested_ids(), vec!["kernel;6.18;amd64;updates"]);
        assert_eq!(plan.change_count(ChangeKind::Update), 1);
        assert_eq!(plan.change_count(ChangeKind::Install), 1);
        assert_eq!(plan.restart, RestartRequirement::SecuritySystem);
    }

    #[test]
    fn simulation_rejects_untrusted_and_oversized_plans() {
        let mut untrusted = PlanCollector::new(&installable_snapshot()).unwrap();
        assert_eq!(
            untrusted
                .apply(package(23, "unsigned;1.0;amd64;third-party"))
                .unwrap_err()
                .kind(),
            ErrorKind::Trust
        );

        let mut oversized = PlanCollector::new(&installable_snapshot()).unwrap();
        for index in 0..MAX_PLAN_CHANGES {
            oversized
                .apply(package(12, &format!("dependency-{index};1;amd64;updates")))
                .unwrap();
        }
        assert_eq!(
            oversized
                .apply(package(12, "one-too-many;1;amd64;updates"))
                .unwrap_err()
                .kind(),
            ErrorKind::Protocol
        );
    }

    #[test]
    fn installability_requires_complete_supported_authority() {
        let mut snapshot = installable_snapshot();
        assert!(snapshot.can_prepare_install());
        snapshot.truncated = true;
        assert!(!snapshot.can_prepare_install());
        snapshot.truncated = false;
        snapshot.install_supported = false;
        assert!(!snapshot.can_prepare_install());
    }

    #[test]
    fn progress_handles_unknown_percentages_and_cancellation() {
        let mut progress = InstallProgress::default();
        progress.set_percentage(87);
        assert_eq!(progress.percentage, Some(87));
        progress.set_percentage(101);
        assert_eq!(progress.percentage, None);
        assert_eq!(
            InstallPhase::from_packagekit(31),
            InstallPhase::WaitingForAuthorization
        );

        let cancellation = Cancellation::default();
        assert!(!cancellation.is_cancelled());
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn installation_collects_unique_packages_and_strongest_restart() {
        let mut collector = InstallCollector::default();
        collector
            .apply(package(10, "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package(11, "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(Event::RestartRequired {
                kind: 4,
                package_id: "kernel;6.18;amd64;updates".into(),
            })
            .unwrap();
        collector
            .apply(Event::RestartRequired {
                kind: 6,
                package_id: "kernel;6.18;amd64;updates".into(),
            })
            .unwrap();
        collector.apply(Event::Finished { exit: 1 }).unwrap();

        let result = collector.finish().unwrap();
        assert_eq!(result.changed_packages, 1);
        assert_eq!(result.restart, RestartRequirement::SecuritySystem);
    }

    #[test]
    fn installation_rejects_an_unbounded_result() {
        let mut collector = InstallCollector::default();
        for index in 0..MAX_PLAN_CHANGES {
            collector
                .apply(package(11, &format!("changed-{index};1;amd64;updates")))
                .unwrap();
        }
        assert_eq!(
            collector
                .apply(package(11, "one-too-many;1;amd64;updates"))
                .unwrap_err()
                .kind(),
            ErrorKind::Protocol
        );
    }

    #[test]
    fn fake_source_preserves_success_and_unavailable_states() {
        let expected = installable_snapshot();
        let success = FakeSource {
            result: Ok(expected.clone()),
        };
        let unavailable = FakeSource {
            result: Err(Error::new(
                ErrorKind::Unavailable,
                "PackageKit is unavailable",
            )),
        };

        assert_eq!(
            futures_lite_for_test(success.snapshot(Request::cached())),
            Ok(expected)
        );
        assert_eq!(
            futures_lite_for_test(unavailable.snapshot(Request::cached()))
                .unwrap_err()
                .kind(),
            ErrorKind::Unavailable
        );
    }

    fn futures_lite_for_test<T>(future: impl Future<Output = T>) -> T {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct Noop;
        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("fixture future unexpectedly pending"),
        }
    }
}
