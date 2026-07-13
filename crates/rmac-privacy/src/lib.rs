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
}
