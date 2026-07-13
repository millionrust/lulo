//! Platform-neutral host sharing state.

use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FirewallState {
    AllowsSsh,
    Inactive,
    ActiveUnverified,
    #[default]
    Unavailable,
}

impl FirewallState {
    pub fn label(self) -> &'static str {
        match self {
            Self::AllowsSsh => "UFW explicitly allows SSH",
            Self::Inactive => "UFW is inactive",
            Self::ActiveUnverified => "UFW is active; SSH allowance was not found",
            Self::Unavailable => "Firewall reachability is unverified",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteLogin {
    pub available: bool,
    pub unit: Option<String>,
    pub active: bool,
    pub service_state: Option<String>,
    pub enabled_at_boot: bool,
    pub unit_file_state: Option<String>,
    pub firewall: FirewallState,
    pub firewall_detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub remote_login: RemoteLogin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Unavailable,
    Authorization,
    Mutation,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
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
    fn set_remote_login(&self, enabled: bool) -> Result<Snapshot, Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firewall_labels_never_overstate_reachability() {
        assert_eq!(
            FirewallState::AllowsSsh.label(),
            "UFW explicitly allows SSH"
        );
        assert!(FirewallState::Inactive.label().contains("inactive"));
        assert!(FirewallState::Unavailable.label().contains("unverified"));
    }
}
