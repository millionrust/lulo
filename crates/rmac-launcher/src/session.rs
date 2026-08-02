use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::engine::{normalize, score};
use crate::{
    Action, ActivationMode, Cancellation, Category, MoveSelection, Privacy, ProviderDescriptor,
    ProviderError, RankedResult, Request, ResultId, SearchResult, DEFAULT_CATEGORY_LIMIT,
    DEFAULT_LIMIT,
};

#[derive(Clone, Debug)]
pub struct Session {
    generation: u64,
    query: String,
    requested: BTreeSet<rmac_shell_settings::ProviderId>,
    categories: BTreeMap<rmac_shell_settings::ProviderId, Category>,
    privacy: BTreeMap<rmac_shell_settings::ProviderId, Privacy>,
    pending: BTreeSet<rmac_shell_settings::ProviderId>,
    batches: BTreeMap<rmac_shell_settings::ProviderId, Vec<SearchResult>>,
    errors: BTreeMap<rmac_shell_settings::ProviderId, ProviderError>,
    ranked: Vec<RankedResult>,
    selected: Option<ResultId>,
    cancellation: Option<Cancellation>,
    limit: usize,
    category_limit: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            generation: 0,
            query: String::new(),
            requested: BTreeSet::new(),
            categories: BTreeMap::new(),
            privacy: BTreeMap::new(),
            pending: BTreeSet::new(),
            batches: BTreeMap::new(),
            errors: BTreeMap::new(),
            ranked: Vec::new(),
            selected: None,
            cancellation: None,
            limit: DEFAULT_LIMIT,
            category_limit: DEFAULT_CATEGORY_LIMIT,
        }
    }
}

impl Session {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    pub fn with_limits(limit: usize, category_limit: usize) -> Self {
        Self {
            limit,
            category_limit,
            ..Self::default()
        }
    }

    pub fn begin(
        &mut self,
        query: impl Into<String>,
        providers: Vec<ProviderDescriptor>,
    ) -> Request {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.query = query.into();
        self.requested = providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect();
        self.categories = providers
            .iter()
            .map(|provider| (provider.id.clone(), provider.category))
            .collect();
        self.privacy = providers
            .iter()
            .map(|provider| (provider.id.clone(), provider.privacy))
            .collect();
        self.pending = self.requested.clone();
        self.batches.clear();
        self.errors.clear();
        self.ranked.clear();
        self.selected = None;
        let cancellation = Cancellation::default();
        self.cancellation = Some(cancellation.clone());
        Request {
            generation: self.generation,
            query: self.query.clone(),
            providers,
            cancellation,
        }
    }

    pub fn cancel(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.pending.clear();
    }

    pub fn apply(
        &mut self,
        generation: u64,
        provider: rmac_shell_settings::ProviderId,
        batch: Result<Vec<SearchResult>, ProviderError>,
    ) -> bool {
        if generation != self.generation
            || !self.requested.contains(&provider)
            || self
                .cancellation
                .as_ref()
                .is_none_or(Cancellation::is_cancelled)
        {
            return false;
        }
        self.pending.remove(&provider);
        match batch {
            Ok(results) => {
                let expected = self.categories.get(&provider).copied();
                let privacy = self.privacy.get(&provider).copied();
                let valid = expected.is_some_and(|expected| {
                    results.iter().all(|result| {
                        result.id.provider == provider
                            && !result.id.local.trim().is_empty()
                            && result.category == expected
                            && privacy.is_some_and(|privacy| {
                                action_allowed(expected, privacy, &result.primary)
                                    && result.alternate.as_ref().is_none_or(|alternate| {
                                        action_allowed(expected, privacy, alternate)
                                    })
                            })
                    })
                });
                if valid {
                    self.batches.insert(provider.clone(), results);
                    self.errors.remove(&provider);
                } else {
                    self.batches.remove(&provider);
                    self.errors.insert(
                        provider,
                        ProviderError {
                            detail: "provider returned an invalid identity, category, or action"
                                .into(),
                        },
                    );
                }
            }
            Err(error) => {
                self.errors.insert(provider, error);
            }
        }
        self.rebuild();
        true
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn results(&self) -> &[RankedResult] {
        &self.ranked
    }

    pub fn errors(&self) -> &BTreeMap<rmac_shell_settings::ProviderId, ProviderError> {
        &self.errors
    }

    pub fn pending(&self) -> &BTreeSet<rmac_shell_settings::ProviderId> {
        &self.pending
    }

    pub fn selected(&self) -> Option<&ResultId> {
        self.selected.as_ref()
    }

    pub fn move_selection(&mut self, direction: MoveSelection) -> Option<&ResultId> {
        if self.ranked.is_empty() {
            self.selected = None;
            return None;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            self.ranked
                .iter()
                .position(|ranked| &ranked.result.id == selected)
        });
        let index = match (current, direction) {
            (Some(index), MoveSelection::Next) => (index + 1) % self.ranked.len(),
            (Some(0), MoveSelection::Previous) | (None, MoveSelection::Previous) => {
                self.ranked.len() - 1
            }
            (Some(index), MoveSelection::Previous) => index - 1,
            (None, MoveSelection::Next) => 0,
        };
        self.selected = Some(self.ranked[index].result.id.clone());
        self.selected.as_ref()
    }

    pub fn select(&mut self, id: &ResultId) -> bool {
        if self.ranked.iter().any(|ranked| &ranked.result.id == id)
            && self.selected.as_ref() != Some(id)
        {
            self.selected = Some(id.clone());
            true
        } else {
            false
        }
    }

    pub fn activation(&self, mode: ActivationMode) -> Option<Action> {
        let selected = self.selected.as_ref()?;
        let result = self
            .ranked
            .iter()
            .find(|ranked| &ranked.result.id == selected)?;
        match mode {
            ActivationMode::Primary => Some(result.result.primary.clone()),
            ActivationMode::Alternate => result.result.alternate.clone(),
        }
    }

    fn rebuild(&mut self) {
        let previous = self.selected.clone();
        let mut unique = BTreeMap::new();
        for result in self.batches.values().flatten() {
            unique
                .entry(result.id.clone())
                .or_insert_with(|| result.clone());
        }
        let query = normalize(&self.query);
        let mut ranked: Vec<_> = unique
            .into_values()
            .filter_map(|result| score(&query, &result).map(|score| RankedResult { result, score }))
            .collect();
        ranked.sort_by_key(|ranked| {
            (
                Reverse(ranked.score),
                ranked.result.category,
                normalize(&ranked.result.title),
                ranked.result.id.clone(),
            )
        });
        let mut category_counts = BTreeMap::new();
        ranked.retain(|ranked| {
            let count = category_counts
                .entry(ranked.result.category)
                .or_insert(0usize);
            if *count >= self.category_limit {
                false
            } else {
                *count += 1;
                true
            }
        });
        ranked.truncate(self.limit);
        self.ranked = ranked;
        self.selected = previous
            .filter(|selected| {
                self.ranked
                    .iter()
                    .any(|ranked| &ranked.result.id == selected)
            })
            .or_else(|| self.ranked.first().map(|ranked| ranked.result.id.clone()));
    }
}

fn action_allowed(category: Category, privacy: Privacy, action: &Action) -> bool {
    match (category, action) {
        (
            Category::Applications,
            Action::LaunchApplication { .. } | Action::RevealApplication { .. },
        )
        | (Category::Settings, Action::OpenSetting { .. })
        | (Category::Calculator, Action::CopyText { .. }) => true,
        (Category::Files, Action::OpenFile { .. } | Action::RevealFile { .. })
        | (Category::Other, Action::OpenFile { .. } | Action::RevealFile { .. }) => {
            privacy.private_content
        }
        (Category::Other, _) => true,
        _ => false,
    }
}
