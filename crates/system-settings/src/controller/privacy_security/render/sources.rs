//! Desktop application source and security-boundary presentation.

use super::*;

impl Settings {
    pub(super) fn append_application_sources(&self, cards: &mut Vec<Div>) {
        cards.push(section_header("Desktop Application Sources"));
        let application_sources = rmac_apps::source_inventory(&self.app_catalog);
        cards.push(card(vec![
            value_row(
                "icons/app-window.svg",
                accent(),
                "Desktop-visible applications".into(),
                application_sources.total().to_string().into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Sandbox package exports".into(),
                format!(
                    "{} Flatpak · {} Snap",
                    application_sources.flatpak, application_sources.snap
                )
                .into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Portable applications".into(),
                format!("{} AppImage", application_sources.appimage).into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Unattributed desktop entries".into(),
                format!(
                    "{} system · {} user · {} other",
                    application_sources.system_desktop_entries,
                    application_sources.user_desktop_entries,
                    application_sources.other_desktop_entries
                )
                .into(),
            ),
        ]));
        cards.push(note_card(
            "Application source counts cover the live desktop-entry catalog. Flatpak and Snap use their exported desktop-entry paths; AppImage uses integration IDs or the launch executable. System and user desktop entries are not claimed to be APT-owned, and command-line-only packages are outside this inventory.",
        ));
        cards.push(note_card(
            "Reset revalidates the selected version-2 application/resource tokens immediately before DeletePermission and proves absence afterward. PermissionStore has no atomic compare-and-delete operation, so a change after that preflight cannot be excluded. Tokens remain uninterpreted because the store does not define their meaning.",
        ));
        cards.push(note_card(
            "Ubuntu coverage and automatic-update values are read-only until a polkit-aware, rollback-safe APT policy editor is reviewed. Package and application source counts describe provenance signals, not repository trust, vulnerability status, coverage, or the security of an individual application.",
        ));
    }
}
