//! Launcher provider registry construction and privacy-scoped file-search policy.

use super::*;

pub(super) fn build_registry(
    application_provider: &rmac_launcher_providers::ApplicationProvider,
    settings: &rmac_shell_settings::ShellSettings,
) -> Result<Arc<Registry>, SharedString> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    build_registry_with_home(application_provider, settings, home)
}

fn build_registry_with_home(
    application_provider: &rmac_launcher_providers::ApplicationProvider,
    settings: &rmac_shell_settings::ShellSettings,
    home: Option<PathBuf>,
) -> Result<Arc<Registry>, SharedString> {
    let mut providers: Vec<Arc<dyn rmac_launcher_providers::Provider>> = vec![
        Arc::new(application_provider.clone()),
        Arc::new(rmac_launcher_providers::SettingsProvider::system_settings()),
        Arc::new(rmac_launcher_providers::CalculatorProvider),
    ];
    if let Some(home) = home {
        let files = rmac_launcher_providers::FileProvider::scoped(
            home,
            &settings.spotlight,
            rmac_launcher_providers::SystemFileSearch,
        )
        .map_err(|_| SharedString::from("File-search scope is unavailable"))?;
        providers.push(Arc::new(files));
    }
    Registry::new(providers)
        .map(Arc::new)
        .map_err(|_| "Search provider registry is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_launcher::Category;

    #[test]
    fn built_in_registry_exposes_all_four_provider_categories() {
        let registry = build_registry_with_home(
            &rmac_launcher_providers::ApplicationProvider::default(),
            &rmac_shell_settings::ShellSettings::default(),
            Some("/home/test".into()),
        )
        .unwrap();
        let categories = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.category)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            categories,
            std::collections::BTreeSet::from([
                Category::Applications,
                Category::Settings,
                Category::Calculator,
                Category::Files,
            ])
        );
    }
}
