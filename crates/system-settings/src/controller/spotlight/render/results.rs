//! Spotlight provider-result projection.

use super::*;

impl Settings {
    pub(super) fn append_spotlight_results(
        &self,
        view: Entity<Self>,
        settings: &rmac_shell_settings::ShellSettings,
        cards: &mut Vec<Div>,
    ) {
        let enabled = !self.shell_settings_busy;
        let applications =
            spotlight_provider_policy(settings, rmac_launcher_providers::APPLICATIONS_PROVIDER);
        let settings_provider =
            spotlight_provider_policy(settings, rmac_launcher_providers::SETTINGS_PROVIDER);
        let files = spotlight_provider_policy(settings, rmac_launcher_providers::FILES_PROVIDER);
        let calculator =
            spotlight_provider_policy(settings, rmac_launcher_providers::CALCULATOR_PROVIDER);
        cards.push(section_header("Search results"));
        cards.push(card(vec![
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::APPLICATIONS_PROVIDER,
                "Applications",
                "Installed desktop applications",
                applications.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::SETTINGS_PROVIDER,
                "System Settings",
                "Destinations and Linux-relevant setting keywords",
                settings_provider.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::FILES_PROVIDER,
                "Files",
                if files.allow_private_content {
                    "On-demand filenames and recent documents"
                } else {
                    "Private-content permission is required"
                },
                files.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::CALCULATOR_PROVIDER,
                "Calculator",
                "Local bounded arithmetic; no scripts or network",
                calculator.enabled,
                enabled,
            ),
        ]));
    }
}
