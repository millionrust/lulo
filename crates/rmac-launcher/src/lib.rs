//! Framework-neutral launcher/Spotlight provider, ranking, and selection model.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 40;
const DEFAULT_CATEGORY_LIMIT: usize = 12;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Category {
    Applications,
    Settings,
    Calculator,
    Files,
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Applications => "Applications",
            Self::Settings => "Settings",
            Self::Calculator => "Calculator",
            Self::Files => "Files",
            Self::Other => "Other",
        }
    }

    fn rank(self) -> u16 {
        match self {
            Self::Applications => 50,
            Self::Settings => 40,
            Self::Calculator => 30,
            Self::Files => 20,
            Self::Other => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Privacy {
    pub private_content: bool,
    pub network: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    pub id: rmac_shell_settings::ProviderId,
    pub category: Category,
    pub privacy: Privacy,
}

pub fn enabled_providers(
    descriptors: &[ProviderDescriptor],
    policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
) -> Vec<ProviderDescriptor> {
    let mut seen = BTreeSet::new();
    descriptors
        .iter()
        .filter(|descriptor| {
            let policy = policies.get(&descriptor.id).cloned().unwrap_or_default();
            policy.enabled
                && (!descriptor.privacy.private_content || policy.allow_private_content)
                && (!descriptor.privacy.network || policy.allow_network)
                && seen.insert(descriptor.id.clone())
        })
        .cloned()
        .collect()
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResultId {
    pub provider: rmac_shell_settings::ProviderId,
    pub local: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    LaunchApplication {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    RevealApplication {
        source: PathBuf,
    },
    OpenSetting {
        pane_id: String,
    },
    OpenFile {
        path: PathBuf,
    },
    RevealFile {
        path: PathBuf,
    },
    CopyText {
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub id: ResultId,
    pub category: Category,
    pub title: String,
    pub subtitle: Option<String>,
    pub primary: Action,
    pub alternate: Option<Action>,
    /// Provider-normalized 0–100 recency/frequency signal, never wall time.
    pub recency_rank: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankedResult {
    pub result: SearchResult,
    pub score: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError {
    pub detail: String,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Debug)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
}

impl Default for Cancellation {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Cancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn flag(&self) -> &AtomicBool {
        &self.cancelled
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub generation: u64,
    pub query: String,
    pub providers: Vec<ProviderDescriptor>,
    pub cancellation: Cancellation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveSelection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationMode {
    Primary,
    Alternate,
}

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

#[derive(Clone, Debug, Default)]
pub struct Launcher {
    open: bool,
    session: Session,
}

impl Launcher {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn open(
        &mut self,
        descriptors: &[ProviderDescriptor],
        policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Request {
        self.open = true;
        self.session
            .begin(String::new(), enabled_providers(descriptors, policies))
    }

    pub fn set_query(
        &mut self,
        query: impl Into<String>,
        descriptors: &[ProviderDescriptor],
        policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        self.open.then(|| {
            self.session
                .begin(query, enabled_providers(descriptors, policies))
        })
    }

    pub fn escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.session.cancel();
        true
    }
}

fn score(query: &str, result: &SearchResult) -> Option<u16> {
    let title = normalize(&result.title);
    let subtitle = result
        .subtitle
        .as_deref()
        .map(normalize)
        .unwrap_or_default();
    let textual = if query.is_empty() {
        100
    } else {
        match_quality(query, &title)
            .or_else(|| match_quality(query, &subtitle).map(|score| score.saturating_sub(80)))?
    };
    Some(
        textual
            .saturating_add(result.category.rank())
            .saturating_add(u16::from(result.recency_rank.min(100))),
    )
}

pub fn query_matches(query: &str, title: &str, subtitle: Option<&str>) -> bool {
    let query = normalize(query);
    query.is_empty()
        || match_quality(&query, &normalize(title)).is_some()
        || subtitle.is_some_and(|subtitle| match_quality(&query, &normalize(subtitle)).is_some())
}

fn match_quality(query: &str, value: &str) -> Option<u16> {
    if value == query {
        Some(1_000)
    } else if value.starts_with(query) {
        Some(850)
    } else if value
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(700)
    } else if value.contains(query) {
        Some(550)
    } else if is_subsequence(query, value) {
        Some(300)
    } else {
        None
    }
}

fn is_subsequence(query: &str, value: &str) -> bool {
    let mut query = query.chars();
    let mut expected = query.next();
    for character in value.chars() {
        if Some(character) == expected {
            expected = query.next();
            if expected.is_none() {
                return true;
            }
        }
    }
    expected.is_none()
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, category: Category, privacy: Privacy) -> ProviderDescriptor {
        ProviderDescriptor {
            id: rmac_shell_settings::ProviderId(id.into()),
            category,
            privacy,
        }
    }

    fn result(provider: &str, local: &str, category: Category, title: &str) -> SearchResult {
        let primary = match category {
            Category::Applications => Action::LaunchApplication {
                app_id: local.into(),
                spec: rmac_apps::LaunchSpec::OpenPath("/Applications/Test.app".into()),
            },
            Category::Settings => Action::OpenSetting {
                pane_id: local.into(),
            },
            Category::Calculator | Category::Other => Action::CopyText { text: title.into() },
            Category::Files => Action::OpenFile {
                path: format!("/home/alex/{local}").into(),
            },
        };
        SearchResult {
            id: ResultId {
                provider: rmac_shell_settings::ProviderId(provider.into()),
                local: local.into(),
            },
            category,
            title: title.into(),
            subtitle: None,
            primary,
            alternate: None,
            recency_rank: 0,
        }
    }

    fn private_files() -> Privacy {
        Privacy {
            private_content: true,
            network: false,
        }
    }

    #[test]
    fn private_and_network_providers_require_explicit_policy() {
        let descriptors = vec![
            provider("apps", Category::Applications, Privacy::default()),
            provider(
                "files",
                Category::Files,
                Privacy {
                    private_content: true,
                    network: false,
                },
            ),
            provider(
                "web",
                Category::Other,
                Privacy {
                    private_content: false,
                    network: true,
                },
            ),
        ];
        assert_eq!(
            enabled_providers(&descriptors, &BTreeMap::new())
                .iter()
                .map(|provider| provider.id.0.as_str())
                .collect::<Vec<_>>(),
            ["apps"]
        );
        let policies = BTreeMap::from([
            (
                rmac_shell_settings::ProviderId("files".into()),
                rmac_shell_settings::ProviderPolicy {
                    allow_private_content: true,
                    ..Default::default()
                },
            ),
            (
                rmac_shell_settings::ProviderId("web".into()),
                rmac_shell_settings::ProviderPolicy {
                    allow_network: true,
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(enabled_providers(&descriptors, &policies).len(), 3);
    }

    #[test]
    fn duplicate_descriptors_and_spoofed_results_are_rejected() {
        let apps = provider("apps", Category::Applications, Privacy::default());
        assert_eq!(
            enabled_providers(&[apps.clone(), apps.clone()], &BTreeMap::new()).len(),
            1
        );
        let mut session = Session::default();
        let request = session.begin("terminal", vec![apps]);
        let spoofed = result("files", "terminal", Category::Files, "Terminal");
        assert!(session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![spoofed]),
        ));
        assert!(session.results().is_empty());
        assert_eq!(session.errors().len(), 1);

        let apps = provider("apps", Category::Applications, Privacy::default());
        let request = session.begin("report", vec![apps]);
        let mut smuggled = result("apps", "report", Category::Applications, "Report");
        smuggled.primary = Action::OpenFile {
            path: "/home/alex/private-report.txt".into(),
        };
        assert!(session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![smuggled]),
        ));
        assert!(session.results().is_empty());
        assert_eq!(session.errors().len(), 1);
    }

    #[test]
    fn exact_and_prefix_matches_rank_deterministically_across_categories() {
        let apps = provider("apps", Category::Applications, Privacy::default());
        let files = provider("files", Category::Files, private_files());
        let mut session = Session::default();
        let request = session.begin("term", vec![apps, files]);
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("files".into()),
            Ok(vec![result(
                "files",
                "1",
                Category::Files,
                "old terminal notes",
            )]),
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![
                result("apps", "terminal", Category::Applications, "Terminal"),
                result("apps", "term", Category::Applications, "Term"),
            ]),
        );
        assert_eq!(
            session
                .results()
                .iter()
                .map(|result| result.result.title.as_str())
                .collect::<Vec<_>>(),
            ["Term", "Terminal", "old terminal notes"]
        );
    }

    #[test]
    fn newer_query_cancels_old_work_and_rejects_stale_batches() {
        let descriptor = provider("apps", Category::Applications, Privacy::default());
        let mut session = Session::default();
        let first = session.begin("term", vec![descriptor.clone()]);
        let second = session.begin("notes", vec![descriptor]);
        assert!(first.cancellation.is_cancelled());
        assert!(!session.apply(
            first.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![result(
                "apps",
                "terminal",
                Category::Applications,
                "Terminal"
            )]),
        ));
        assert!(session.apply(
            second.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![result(
                "apps",
                "notes",
                Category::Applications,
                "Notes"
            )]),
        ));
        assert_eq!(session.results()[0].result.title, "Notes");
    }

    #[test]
    fn provider_failure_preserves_other_results_and_exposes_error() {
        let mut session = Session::default();
        let request = session.begin(
            "term",
            vec![
                provider("apps", Category::Applications, Privacy::default()),
                provider("files", Category::Files, private_files()),
            ],
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![result(
                "apps",
                "terminal",
                Category::Applications,
                "Terminal",
            )]),
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("files".into()),
            Err(ProviderError {
                detail: "search cancelled by mount loss".into(),
            }),
        );
        assert_eq!(session.results().len(), 1);
        assert_eq!(session.errors().len(), 1);
        assert!(session.pending().is_empty());
    }

    #[test]
    fn selection_wraps_and_survives_later_provider_batches() {
        let mut session = Session::default();
        let request = session.begin(
            "",
            vec![
                provider("apps", Category::Applications, Privacy::default()),
                provider("settings", Category::Settings, Privacy::default()),
            ],
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok(vec![result(
                "apps",
                "terminal",
                Category::Applications,
                "Terminal",
            )]),
        );
        let selected = session.selected().cloned();
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("settings".into()),
            Ok(vec![result(
                "settings",
                "sound",
                Category::Settings,
                "Sound",
            )]),
        );
        assert_eq!(session.selected(), selected.as_ref());
        session.move_selection(MoveSelection::Previous);
        assert_eq!(
            session.selected().map(|id| id.local.as_str()),
            Some("sound")
        );
        session.move_selection(MoveSelection::Next);
        assert_eq!(session.selected(), selected.as_ref());
    }

    #[test]
    fn category_cap_prevents_one_provider_from_crowding_out_peers() {
        let mut session = Session::with_limits(10, 2);
        let request = session.begin(
            "",
            vec![
                provider("apps", Category::Applications, Privacy::default()),
                provider("settings", Category::Settings, Privacy::default()),
            ],
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("apps".into()),
            Ok((0..6)
                .map(|index| {
                    result(
                        "apps",
                        &format!("app-{index}"),
                        Category::Applications,
                        &format!("App {index}"),
                    )
                })
                .collect()),
        );
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("settings".into()),
            Ok(vec![result(
                "settings",
                "sound",
                Category::Settings,
                "Sound",
            )]),
        );
        assert_eq!(session.results().len(), 3);
        assert!(session
            .results()
            .iter()
            .any(|result| result.result.category == Category::Settings));
    }

    #[test]
    fn alternate_activation_is_explicit_and_never_falls_back() {
        let mut file = result("files", "report", Category::Files, "Report");
        file.primary = Action::OpenFile {
            path: PathBuf::from("/home/alex/Report.txt"),
        };
        file.alternate = Some(Action::RevealFile {
            path: PathBuf::from("/home/alex/Report.txt"),
        });
        let descriptor = provider("files", Category::Files, private_files());
        let mut session = Session::default();
        let request = session.begin("report", vec![descriptor]);
        session.apply(
            request.generation,
            rmac_shell_settings::ProviderId("files".into()),
            Ok(vec![file]),
        );
        assert!(matches!(
            session.activation(ActivationMode::Alternate),
            Some(Action::RevealFile { .. })
        ));
    }

    #[test]
    fn escape_closes_overlay_and_cancels_provider_work() {
        let descriptors = [provider("apps", Category::Applications, Privacy::default())];
        let mut launcher = Launcher::default();
        let request = launcher.open(&descriptors, &BTreeMap::new());
        assert!(launcher.is_open());
        assert!(launcher.escape());
        assert!(!launcher.is_open());
        assert!(request.cancellation.is_cancelled());
        assert!(!launcher.escape());
    }
}
