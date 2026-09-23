use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::engine::{normalize, score};
use crate::{
    Action, ActivationMode, ApplicationGroup, Cancellation, Category, Learning, MoveSelection,
    Privacy, ProviderDescriptor, ProviderError, RankedResult, Request, ResultId, SearchResult,
    DEFAULT_CATEGORY_LIMIT, DEFAULT_LIMIT,
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
    /// The person moved or picked the selection this query; until then
    /// it follows the first row as batches arrive.
    chosen: bool,
    cancellation: Option<Cancellation>,
    limit: usize,
    category_limit: usize,
    /// What earlier choices taught, and the time it is judged at.
    learning: Option<Arc<Learning>>,
    now: u64,
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
            chosen: false,
            cancellation: None,
            limit: DEFAULT_LIMIT,
            category_limit: DEFAULT_CATEGORY_LIMIT,
            learning: None,
            now: 0,
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

    /// Rank with what earlier choices taught (`now` in Unix seconds). Takes
    /// effect for the next batch or query.
    pub fn set_learning(&mut self, learning: Arc<Learning>, now: u64) {
        self.learning = Some(learning);
        self.now = now;
        if !self.batches.is_empty() {
            self.rebuild();
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
        self.chosen = false;
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
        self.chosen = true;
        self.selected.as_ref()
    }

    /// Move only through results in a Tahoe-style Spotlight browse mode.
    pub fn move_selection_in_category(
        &mut self,
        category: Category,
        direction: MoveSelection,
    ) -> Option<&ResultId> {
        let matching = self
            .ranked
            .iter()
            .enumerate()
            .filter_map(|(index, ranked)| (ranked.result.category == category).then_some(index))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            self.selected = None;
            return None;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            matching
                .iter()
                .position(|index| &self.ranked[*index].result.id == selected)
        });
        let position = match (current, direction) {
            (Some(position), MoveSelection::Next) => (position + 1) % matching.len(),
            (Some(0), MoveSelection::Previous) | (None, MoveSelection::Previous) => {
                matching.len() - 1
            }
            (Some(position), MoveSelection::Previous) => position - 1,
            (None, MoveSelection::Next) => 0,
        };
        self.selected = Some(self.ranked[matching[position]].result.id.clone());
        self.chosen = true;
        self.selected.as_ref()
    }

    /// Move only through visible applications in one Apps browse category.
    pub fn move_selection_in_application_group(
        &mut self,
        group: ApplicationGroup,
        direction: MoveSelection,
    ) -> Option<&ResultId> {
        let matching = self
            .ranked
            .iter()
            .enumerate()
            .filter_map(|(index, ranked)| {
                (ranked.result.category == Category::Applications
                    && ranked.result.application_group == Some(group))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        if matching.is_empty() {
            self.selected = None;
            return None;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            matching
                .iter()
                .position(|index| &self.ranked[*index].result.id == selected)
        });
        let position = match (current, direction) {
            (Some(position), MoveSelection::Next) => (position + 1) % matching.len(),
            (Some(0), MoveSelection::Previous) | (None, MoveSelection::Previous) => {
                matching.len() - 1
            }
            (Some(position), MoveSelection::Previous) => position - 1,
            (None, MoveSelection::Next) => 0,
        };
        self.selected = Some(self.ranked[matching[position]].result.id.clone());
        self.chosen = true;
        self.selected.as_ref()
    }

    /// Command-Down / Command-Up: the first row of the next section, or of
    /// the current section (then the previous one) going up. Sections are
    /// the top hit alone, then each run of one category, as on macOS. The
    /// selection does not wrap.
    pub fn move_selection_by_section(&mut self, direction: MoveSelection) -> Option<&ResultId> {
        let categories = self
            .ranked
            .iter()
            .map(|ranked| ranked.result.category)
            .collect::<Vec<_>>();
        let starts = section_starts(&categories);
        if starts.is_empty() {
            self.selected = None;
            return None;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            self.ranked
                .iter()
                .position(|ranked| &ranked.result.id == selected)
        });
        let index = match (current, direction) {
            (None, MoveSelection::Next) => 0,
            (None, MoveSelection::Previous) => starts[starts.len() - 1],
            (Some(index), MoveSelection::Next) => starts
                .iter()
                .copied()
                .find(|start| *start > index)
                .unwrap_or(index),
            (Some(index), MoveSelection::Previous) => starts
                .iter()
                .rev()
                .copied()
                .find(|start| *start < index)
                .unwrap_or(index),
        };
        self.selected = Some(self.ranked[index].result.id.clone());
        self.chosen = true;
        self.selected.as_ref()
    }

    pub fn select(&mut self, id: &ResultId) -> bool {
        if self.ranked.iter().any(|ranked| &ranked.result.id == id)
            && self.selected.as_ref() != Some(id)
        {
            self.selected = Some(id.clone());
            self.chosen = true;
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
        let learning = self.learning.clone();
        let now = self.now;
        let mut ranked: Vec<_> = unique
            .into_values()
            .filter_map(|result| {
                let learned = learning
                    .as_ref()
                    .map_or(0, |learning| learning.boost(&query, &result.id, now));
                score(&query, &result, learned).map(|score| RankedResult { result, score })
            })
            .collect();
        if query.is_empty() {
            // The empty-query renderer presents the application grid before
            // Suggestions. Keep the controller's arrow/selection order equal
            // to that visual reading order while preserving ranking within
            // each region.
            ranked.sort_by_key(|ranked| {
                (
                    ranked.result.category != Category::Applications,
                    Reverse(ranked.score),
                    ranked.result.category,
                    normalize(&ranked.result.title),
                    ranked.result.id.clone(),
                )
            });
        } else {
            // Answers lead (the Mac's answer card), the "Search in" rows
            // close the list, and everything else is ranked between them.
            ranked.sort_by_key(|ranked| {
                (
                    list_region(ranked.result.category),
                    Reverse(ranked.score),
                    ranked.result.category,
                    normalize(&ranked.result.title),
                    ranked.result.id.clone(),
                )
            });
        }
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
        // The "Search in" rows survive the overall limit: they are the way
        // on when the list is cut short.
        let footer = ranked
            .iter()
            .filter(|ranked| ranked.result.category == Category::SearchIn)
            .cloned()
            .collect::<Vec<_>>();
        ranked.retain(|ranked| ranked.result.category != Category::SearchIn);
        ranked.truncate(self.limit.saturating_sub(footer.len()));
        ranked.extend(footer);
        self.ranked = ranked;
        // A selection the person made stays put. One that only followed an
        // earlier first row stays too (no flicker as batches arrive), except
        // that an answer arriving takes the top, and the "Search in" row
        // never keeps it once real results exist.
        let answer_first = self
            .ranked
            .first()
            .is_some_and(|ranked| ranked.result.category.is_answer());
        let chosen = self.chosen;
        self.selected = previous
            .filter(|selected| {
                self.ranked
                    .iter()
                    .find(|ranked| &ranked.result.id == selected)
                    .is_some_and(|ranked| {
                        chosen || (!answer_first && ranked.result.category != Category::SearchIn)
                    })
            })
            .or_else(|| self.ranked.first().map(|ranked| ranked.result.id.clone()));
    }
}

/// Where a category sits in a query's list: answers, results, "Search in".
fn list_region(category: Category) -> u8 {
    if category.is_answer() {
        0
    } else if category == Category::SearchIn {
        2
    } else {
        1
    }
}

/// First index of each section: the top hit alone, then each run of one
/// category.
pub fn section_starts(categories: &[Category]) -> Vec<usize> {
    let mut starts = Vec::new();
    for (index, category) in categories.iter().enumerate() {
        if index <= 1 || categories[index - 1] != *category {
            starts.push(index);
        }
    }
    starts
}

fn action_allowed(category: Category, privacy: Privacy, action: &Action) -> bool {
    match (category, action) {
        (
            Category::Applications,
            Action::LaunchApplication { .. } | Action::RevealApplication { .. },
        )
        | (Category::Settings, Action::OpenSetting { .. })
        | (
            Category::Calculator | Category::Clock | Category::Dictionary,
            Action::CopyText { .. },
        )
        | (Category::SearchIn, Action::SearchFiles { .. }) => true,
        (Category::Files, Action::OpenFile { .. } | Action::RevealFile { .. })
        | (Category::Other, Action::OpenFile { .. } | Action::RevealFile { .. }) => {
            privacy.private_content
        }
        (Category::Other, _) => true,
        _ => false,
    }
}
