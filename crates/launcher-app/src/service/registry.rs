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
        currency_provider(),
        Arc::new(rmac_launcher_providers::WorldClockProvider::new(
            super::world_clock::SystemClock,
        )),
        Arc::new(rmac_launcher_providers::SearchInFilesProvider),
    ];
    // Definitions only when a dictd dictionary is installed (dict-wn,
    // dict-gcide); Ubuntu ships none by default.
    let dictionary = rmac_launcher_providers::DictionaryProvider::system();
    if dictionary.is_available() {
        providers.push(Arc::new(dictionary));
    }
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

/// One rate cache for the life of the service, so a fetched day of rates
/// is shared by every overlay.
fn currency_provider() -> Arc<dyn rmac_launcher_providers::Provider> {
    static PROVIDER: std::sync::OnceLock<
        Arc<rmac_launcher_providers::CurrencyProvider<rmac_launcher_providers::EcbRates>>,
    > = std::sync::OnceLock::new();
    PROVIDER
        .get_or_init(|| {
            Arc::new(rmac_launcher_providers::CurrencyProvider::new(
                rmac_launcher_providers::EcbRates::new(
                    rmac_launcher_providers::EcbRates::default_cache(),
                ),
            ))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_launcher::Category;

    #[test]
    fn built_in_registry_exposes_every_answer_category() {
        let registry = build_registry_with_home(
            &rmac_launcher_providers::ApplicationProvider::default(),
            &rmac_shell_settings::ShellSettings::default(),
            Some("/home/test".into()),
        )
        .unwrap();
        let mut categories = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.category)
            .collect::<std::collections::BTreeSet<_>>();
        // Present only where a dictd dictionary is installed.
        categories.remove(&Category::Dictionary);
        assert_eq!(
            categories,
            std::collections::BTreeSet::from([
                Category::Applications,
                Category::Settings,
                Category::Calculator,
                Category::Clock,
                Category::Files,
                Category::SearchIn,
            ])
        );
    }
}
