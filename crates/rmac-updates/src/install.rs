//! Install progress, result, cancellation, and watch model.

use super::*;

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
