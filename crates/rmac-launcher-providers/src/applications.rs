//! Installed-application launcher provider.

use super::*;

#[derive(Clone, Debug, Default)]
pub struct ApplicationProvider {
    state: Arc<RwLock<ApplicationState>>,
}

#[derive(Debug, Default)]
struct ApplicationState {
    catalog: Vec<rmac_apps::Application>,
    revision: u64,
}

impl ApplicationProvider {
    pub fn new(catalog: Vec<rmac_apps::Application>) -> Self {
        let provider = Self::default();
        provider.replace_catalog(catalog);
        provider
    }

    pub fn discover() -> Result<Self, ProviderError> {
        rmac_apps::discover()
            .map(Self::new)
            .map_err(|error| ProviderError {
                detail: error.to_string(),
            })
    }

    /// Atomically replace the installed-application snapshot. Existing clones
    /// observe the same revision, and identical discoveries do not churn an
    /// open launcher query.
    pub fn replace_catalog(&self, catalog: Vec<rmac_apps::Application>) -> bool {
        let catalog = normalized_catalog(catalog);
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.catalog == catalog {
            return false;
        }
        state.catalog = catalog;
        state.revision = state.revision.wrapping_add(1).max(1);
        true
    }

    pub fn revision(&self) -> u64 {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision
    }

    pub fn refresh(&self) -> Result<bool, ProviderError> {
        let catalog = rmac_apps::discover().map_err(|error| ProviderError {
            detail: error.to_string(),
        })?;
        Ok(self.replace_catalog(catalog))
    }
}

fn normalized_catalog(catalog: Vec<rmac_apps::Application>) -> Vec<rmac_apps::Application> {
    let mut seen = BTreeSet::new();
    catalog
        .into_iter()
        .filter(|application| {
            !application.id.trim().is_empty()
                && !application.name.trim().is_empty()
                && seen.insert(application.id.clone())
        })
        .collect()
}

impl Provider for ApplicationProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            APPLICATIONS_PROVIDER,
            Category::Applications,
            Privacy::default(),
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let catalog = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .catalog
            .clone();
        let mut results = Vec::new();
        for application in &catalog {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let subtitle = application.generic_name.clone().or_else(|| {
                (!application.categories.is_empty()).then(|| application.categories.join(", "))
            });
            let searchable = application.searchable_text();
            if !rmac_launcher::query_matches(query, &application.name, Some(&searchable)) {
                continue;
            }
            results.push(SearchResult {
                id: ResultId {
                    provider: provider_id(APPLICATIONS_PROVIDER),
                    local: application.id.clone(),
                },
                category: Category::Applications,
                title: application.name.clone(),
                subtitle,
                icon: application.icon.clone(),
                primary: Action::LaunchApplication {
                    app_id: application.id.clone(),
                    spec: application.launch.clone(),
                },
                alternate: Some(Action::RevealApplication {
                    source: application.source.clone(),
                }),
                recency_rank: 0,
            });
            if results.len() == PROVIDER_LIMIT {
                break;
            }
        }
        Ok(results)
    }
}
