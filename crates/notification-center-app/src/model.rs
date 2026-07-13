use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui::SharedString;
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord};

#[derive(Clone)]
pub(crate) struct ApplicationIdentity {
    pub(crate) name: SharedString,
    pub(crate) icon: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Busy {
    ClearAll,
    ClearApp(String),
    DisableApp(String),
    Invoke(NotificationId, ActionSelection),
}

pub(crate) struct RecordGroup<'a> {
    pub(crate) app_id: &'a str,
    pub(crate) records: Vec<&'a HistoryRecord>,
}

pub(crate) fn application_identities(
    catalog: Vec<rmac_apps::Application>,
) -> BTreeMap<String, ApplicationIdentity> {
    let mut identities = BTreeMap::new();
    for application in catalog {
        let identity = ApplicationIdentity {
            name: application.name.into(),
            icon: application.icon,
        };
        identities.insert(application.id.clone(), identity.clone());
        if let Some(alias) = application.id.strip_suffix(".desktop") {
            identities.entry(alias.to_owned()).or_insert(identity);
        }
    }
    identities
}

pub(crate) fn fallback_app_name(app_id: &str) -> String {
    if app_id.is_empty() {
        "Application".into()
    } else {
        app_id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_application_identity_is_never_guessed() {
        assert_eq!(
            fallback_app_name("org.example.Private.desktop"),
            "org.example.Private.desktop"
        );
        assert_eq!(fallback_app_name(""), "Application");
    }
}
