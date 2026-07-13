//! Platform-neutral host sharing state.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FirewallState {
    Allows,
    Inactive,
    ActiveUnverified,
    #[default]
    Unavailable,
}

impl FirewallState {
    pub fn label(self, service: &str) -> String {
        match self {
            Self::Allows => format!("UFW explicitly allows {service}"),
            Self::Inactive => "UFW is inactive".into(),
            Self::ActiveUnverified => {
                format!("UFW is active; {service} allowance was not found")
            }
            Self::Unavailable => "Firewall reachability is unverified".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub name: String,
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
pub struct FileSharing {
    pub available: bool,
    pub unit: Option<String>,
    pub active: bool,
    pub service_state: Option<String>,
    pub enabled_at_boot: bool,
    pub unit_file_state: Option<String>,
    pub shares: Vec<Share>,
    pub shares_truncated: bool,
    pub configuration_error: Option<String>,
    pub firewall: FirewallState,
    pub firewall_detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub remote_login: RemoteLogin,
    pub file_sharing: FileSharing,
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
            FirewallState::Allows.label("SSH"),
            "UFW explicitly allows SSH"
        );
        assert!(FirewallState::Inactive.label("SSH").contains("inactive"));
        assert!(FirewallState::Unavailable
            .label("Samba")
            .contains("unverified"));
    }
}
