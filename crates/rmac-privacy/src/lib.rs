//! Domain model for privacy authorities surfaced by System Settings.

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortalResource {
    Camera,
    Microphone,
}

impl PortalResource {
    pub fn id(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Microphone => "Microphone",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalDecision {
    pub resource: PortalResource,
    pub app_id: String,
    pub permissions: Vec<String>,
}

impl PortalDecision {
    pub fn summary(&self) -> String {
        if self.permissions.is_empty() {
            "Stored decision".to_string()
        } else {
            self.permissions.join(", ")
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub version: u32,
    pub can_reset: bool,
    pub decisions: Vec<PortalDecision>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageSources {
    pub installed: u64,
    pub main: u64,
    pub restricted: u64,
    pub universe: u64,
    pub multiverse: u64,
    pub esm_apps: u64,
    pub esm_infra: u64,
    pub third_party: u64,
    pub unknown: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProStatus {
    pub attached: bool,
    pub contract_valid: bool,
    pub contract_status: Option<String>,
    pub contract_remaining_days: i64,
    pub enabled_services: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutomaticUpdates {
    pub running: bool,
    pub apt_timer_enabled: bool,
    pub periodic_job_enabled: bool,
    pub package_list_frequency_days: u64,
    pub upgrade_frequency_days: u64,
    pub allowed_origins: Vec<String>,
    pub last_run: Option<String>,
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseSupport {
    pub series: String,
    pub days_remaining: i64,
}

impl ReleaseSupport {
    pub fn supported(&self) -> bool {
        self.days_remaining >= 0
    }
}

impl AutomaticUpdates {
    pub fn fully_enabled(&self) -> bool {
        self.running
            && self.apt_timer_enabled
            && self.periodic_job_enabled
            && self.upgrade_frequency_days > 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecurityCoverageSnapshot {
    pub pro_client_available: bool,
    pub release_support: Option<ReleaseSupport>,
    pub package_sources: Option<PackageSources>,
    pub pro: Option<ProStatus>,
    pub automatic_updates: Option<AutomaticUpdates>,
    pub issues: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_summary_preserves_the_authoritative_tokens() {
        let decision = PortalDecision {
            resource: PortalResource::Camera,
            app_id: "org.example.Camera".into(),
            permissions: vec!["yes".into(), "session".into()],
        };
        assert_eq!(decision.summary(), "yes, session");
    }

    #[test]
    fn automatic_updates_require_every_authoritative_gate() {
        let enabled = AutomaticUpdates {
            running: true,
            apt_timer_enabled: true,
            periodic_job_enabled: true,
            upgrade_frequency_days: 1,
            ..AutomaticUpdates::default()
        };
        assert!(enabled.fully_enabled());

        let mut disabled = enabled;
        disabled.periodic_job_enabled = false;
        assert!(!disabled.fully_enabled());
    }

    #[test]
    fn release_support_includes_the_final_supported_day() {
        assert!(ReleaseSupport {
            series: "resolute".into(),
            days_remaining: 0,
        }
        .supported());
        assert!(!ReleaseSupport {
            series: "plucky".into(),
            days_remaining: -1,
        }
        .supported());
    }
}
