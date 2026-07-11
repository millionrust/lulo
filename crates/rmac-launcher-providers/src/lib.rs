//! Local application, setting, file, and calculator launcher providers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rmac_launcher::{
    Action, Cancellation, Category, Privacy, ProviderDescriptor, ProviderError, Request, ResultId,
    SearchResult,
};

pub const APPLICATIONS_PROVIDER: &str = "applications";
pub const SETTINGS_PROVIDER: &str = "settings";
pub const FILES_PROVIDER: &str = "files";
pub const CALCULATOR_PROVIDER: &str = "calculator";

const PROVIDER_LIMIT: usize = 100;

pub trait Provider: Send + Sync + 'static {
    fn descriptor(&self) -> ProviderDescriptor;
    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Batch {
    pub generation: u64,
    pub provider: rmac_shell_settings::ProviderId,
    pub results: Result<Vec<SearchResult>, ProviderError>,
}

/// Run one provider only if its exact descriptor was admitted into this
/// privacy-filtered request. Call this function on a background executor.
pub fn execute(request: &Request, provider: &(impl Provider + ?Sized)) -> Option<Batch> {
    let descriptor = provider.descriptor();
    if request.cancellation.is_cancelled()
        || !request
            .providers
            .iter()
            .any(|admitted| admitted == &descriptor)
    {
        return None;
    }
    let results = provider.search(&request.query, &request.cancellation);
    (!request.cancellation.is_cancelled()).then_some(Batch {
        generation: request.generation,
        provider: descriptor.id,
        results,
    })
}

#[derive(Clone, Debug, Default)]
pub struct ApplicationProvider {
    catalog: Vec<rmac_apps::Application>,
}

impl ApplicationProvider {
    pub fn new(catalog: Vec<rmac_apps::Application>) -> Self {
        Self { catalog }
    }

    pub fn discover() -> Result<Self, ProviderError> {
        rmac_apps::discover()
            .map(Self::new)
            .map_err(|error| ProviderError {
                detail: error.to_string(),
            })
    }
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
        let mut results = Vec::new();
        for application in &self.catalog {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let subtitle =
                (!application.categories.is_empty()).then(|| application.categories.join(", "));
            if !rmac_launcher::query_matches(query, &application.name, subtitle.as_deref()) {
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
                primary: Action::LaunchApplication {
                    app_id: application.id.clone(),
                    spec: application.launch.clone(),
                },
                alternate: None,
                recency_rank: 0,
            });
            if results.len() == PROVIDER_LIMIT {
                break;
            }
        }
        Ok(results)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingEntry {
    pub pane_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsProvider {
    entries: Vec<SettingEntry>,
}

impl SettingsProvider {
    pub fn new(entries: Vec<SettingEntry>) -> Self {
        Self { entries }
    }
}

impl Provider for SettingsProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(SETTINGS_PROVIDER, Category::Settings, Privacy::default())
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let mut seen = BTreeSet::new();
        let mut results = Vec::new();
        for entry in &self.entries {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            if entry.pane_id.trim().is_empty()
                || !seen.insert(entry.pane_id.clone())
                || !(rmac_launcher::query_matches(query, &entry.title, entry.subtitle.as_deref())
                    || entry
                        .keywords
                        .iter()
                        .any(|keyword| rmac_launcher::query_matches(query, keyword, None)))
            {
                continue;
            }
            results.push(SearchResult {
                id: ResultId {
                    provider: provider_id(SETTINGS_PROVIDER),
                    local: entry.pane_id.clone(),
                },
                category: Category::Settings,
                title: entry.title.clone(),
                subtitle: entry.subtitle.clone(),
                primary: Action::OpenSetting {
                    pane_id: entry.pane_id.clone(),
                },
                alternate: None,
                recency_rank: 0,
            });
            if results.len() == PROVIDER_LIMIT {
                break;
            }
        }
        Ok(results)
    }
}

pub trait FileSearch: Send + Sync + 'static {
    fn filenames(
        &self,
        root: &Path,
        query: &str,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error>;

    fn recents(
        &self,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemFileSearch;

impl FileSearch for SystemFileSearch {
    fn filenames(
        &self,
        root: &Path,
        query: &str,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error> {
        rmac_search::filenames(root, query, options)
    }

    fn recents(
        &self,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error> {
        rmac_search::recents(options)
    }
}

#[derive(Clone, Debug)]
pub struct FileProvider<S = SystemFileSearch> {
    root: PathBuf,
    search: S,
}

impl FileProvider<SystemFileSearch> {
    pub fn system(root: PathBuf) -> Self {
        Self {
            root,
            search: SystemFileSearch,
        }
    }
}

impl<S> FileProvider<S> {
    pub fn new(root: PathBuf, search: S) -> Self {
        Self { root, search }
    }
}

impl<S: FileSearch> Provider for FileProvider<S> {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            FILES_PROVIDER,
            Category::Files,
            Privacy {
                private_content: true,
                network: false,
            },
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if !self.root.is_absolute() {
            return Err(ProviderError {
                detail: "file search root must be absolute".into(),
            });
        }
        let mut options = rmac_search::Options::new(cancellation.flag());
        options.limit = PROVIDER_LIMIT;
        let paths = if query.trim().is_empty() {
            self.search.recents(options)
        } else {
            self.search.filenames(&self.root, query, options)
        }
        .map_err(|error| ProviderError {
            detail: error.to_string(),
        })?;
        let mut seen = BTreeSet::new();
        Ok(paths
            .into_iter()
            .filter(|path| path.is_absolute() && seen.insert(path.clone()))
            .filter_map(|path| file_result(path, query))
            .take(PROVIDER_LIMIT)
            .collect())
    }
}

fn file_result(path: PathBuf, query: &str) -> Option<SearchResult> {
    let title = path.file_name()?.to_string_lossy().into_owned();
    let subtitle = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned());
    if !rmac_launcher::query_matches(query, &title, subtitle.as_deref()) {
        return None;
    }
    let local = path.to_string_lossy().into_owned();
    Some(SearchResult {
        id: ResultId {
            provider: provider_id(FILES_PROVIDER),
            local,
        },
        category: Category::Files,
        title,
        subtitle,
        primary: Action::OpenFile { path: path.clone() },
        alternate: Some(Action::RevealFile { path }),
        recency_rank: 0,
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CalculatorProvider;

impl Provider for CalculatorProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            CALCULATOR_PROVIDER,
            Category::Calculator,
            Privacy::default(),
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let Some(value) = evaluate(query) else {
            return Ok(Vec::new());
        };
        let text = format_number(value);
        Ok(vec![SearchResult {
            id: ResultId {
                provider: provider_id(CALCULATOR_PROVIDER),
                local: query.trim().into(),
            },
            category: Category::Calculator,
            title: text.clone(),
            subtitle: Some(query.trim().into()),
            primary: Action::CopyText { text },
            alternate: None,
            recency_rank: 0,
        }])
    }
}

fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    if input.is_empty()
        || input.len() > 256
        || !input
            .chars()
            .any(|character| matches!(character, '+' | '-' | '*' | '/'))
    {
        return None;
    }
    let mut parser = Parser::new(input);
    let value = parser.expression()?;
    parser.skip_whitespace();
    (parser.position == parser.input.len() && value.is_finite()).then_some(value)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            self.skip_whitespace();
            if self.consume(b'+') {
                value += self.term()?;
            } else if self.consume(b'-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        loop {
            self.skip_whitespace();
            if self.consume(b'*') {
                value *= self.factor()?;
            } else if self.consume(b'/') {
                let divisor = self.factor()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    fn factor(&mut self) -> Option<f64> {
        self.skip_whitespace();
        if self.consume(b'+') {
            return self.factor();
        }
        if self.consume(b'-') {
            return self.factor().map(|value| -value);
        }
        if self.consume(b'(') {
            let value = self.expression()?;
            self.skip_whitespace();
            return self.consume(b')').then_some(value);
        }
        self.number()
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_whitespace();
        let start = self.position;
        let mut decimal = false;
        while let Some(byte) = self.input.get(self.position) {
            match byte {
                b'0'..=b'9' => self.position += 1,
                b'.' if !decimal => {
                    decimal = true;
                    self.position += 1;
                }
                _ => break,
            }
        }
        (self.position > start)
            .then(|| std::str::from_utf8(&self.input[start..self.position]).ok())
            .flatten()?
            .parse()
            .ok()
    }

    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

fn format_number(value: f64) -> String {
    let formatted = format!("{value:.10}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

fn descriptor(id: &str, category: Category, privacy: Privacy) -> ProviderDescriptor {
    ProviderDescriptor {
        id: provider_id(id),
        category,
        privacy,
    }
}

fn provider_id(id: &str) -> rmac_shell_settings::ProviderId {
    rmac_shell_settings::ProviderId(id.into())
}

fn cancelled() -> ProviderError {
    ProviderError {
        detail: "provider cancelled".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: vec!["Utility".into()],
            launch: rmac_apps::LaunchSpec::Command {
                program: id.into(),
                args: vec!["--new".into()],
                working_dir: None,
                terminal: false,
            },
        }
    }

    #[test]
    fn application_provider_preserves_exact_launch_spec() {
        let provider = ApplicationProvider::new(vec![application("terminal.desktop", "Terminal")]);
        let results = provider
            .search("term", &Cancellation::default())
            .expect("search succeeds");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            &results[0].primary,
            Action::LaunchApplication { app_id, spec: rmac_apps::LaunchSpec::Command { args, .. } }
                if app_id == "terminal.desktop" && args == &["--new"]
        ));
    }

    #[test]
    fn settings_provider_matches_keywords_and_deduplicates_panes() {
        let provider = SettingsProvider::new(vec![
            SettingEntry {
                pane_id: "sound".into(),
                title: "Sound".into(),
                subtitle: Some("Output and input".into()),
                keywords: vec!["volume".into()],
            },
            SettingEntry {
                pane_id: "sound".into(),
                title: "Duplicate".into(),
                subtitle: None,
                keywords: Vec::new(),
            },
        ]);
        let results = provider
            .search("volume", &Cancellation::default())
            .expect("search succeeds");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.local, "sound");
    }

    #[derive(Default)]
    struct FakeFileSearch {
        paths: Vec<PathBuf>,
    }

    impl FileSearch for FakeFileSearch {
        fn filenames(
            &self,
            _: &Path,
            _: &str,
            options: rmac_search::Options<'_>,
        ) -> Result<Vec<PathBuf>, rmac_search::Error> {
            if options.cancel.load(Ordering::Acquire) {
                return Err(rmac_search::Error::Cancelled);
            }
            Ok(self.paths.clone())
        }

        fn recents(&self, _: rmac_search::Options<'_>) -> Result<Vec<PathBuf>, rmac_search::Error> {
            Ok(self.paths.clone())
        }
    }

    #[test]
    fn file_provider_is_private_deduplicated_and_revealable() {
        let provider = FileProvider::new(
            PathBuf::from("/home/alex"),
            FakeFileSearch {
                paths: vec![
                    PathBuf::from("/home/alex/Report.txt"),
                    PathBuf::from("/home/alex/Report.txt"),
                    PathBuf::from("relative.txt"),
                ],
            },
        );
        assert!(provider.descriptor().privacy.private_content);
        let results = provider
            .search("report", &Cancellation::default())
            .expect("search succeeds");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].alternate,
            Some(Action::RevealFile { .. })
        ));
    }

    #[test]
    fn provider_execution_requires_exact_admission_and_honors_cancellation() {
        let provider = CalculatorProvider;
        let descriptor = provider.descriptor();
        let mut session = rmac_launcher::Session::default();
        let admitted = session.begin("2+2", vec![descriptor]);
        assert!(execute(&admitted, &provider).is_some());

        let unadmitted = session.begin("2+2", Vec::new());
        assert!(execute(&unadmitted, &provider).is_none());
        admitted.cancellation.cancel();
        assert!(execute(&admitted, &provider).is_none());
    }

    #[test]
    fn calculator_is_bounded_and_respects_precedence_parentheses_and_unary() {
        let provider = CalculatorProvider;
        for (expression, expected) in [
            ("2 + 3 * 4", "14"),
            ("(2 + 3) * 4", "20"),
            ("-5 / 2", "-2.5"),
        ] {
            let results = provider
                .search(expression, &Cancellation::default())
                .expect("calculator succeeds");
            assert_eq!(results[0].title, expected);
        }
        let too_long = "1+".repeat(200);
        for expression in ["1 / 0", "2 +", "hello", too_long.as_str()] {
            assert!(provider
                .search(expression, &Cancellation::default())
                .expect("invalid expression is not an error")
                .is_empty());
        }
    }

    #[test]
    fn cancellation_flag_is_compatible_with_search_options() {
        let cancellation = Cancellation::default();
        let options = rmac_search::Options::new(cancellation.flag());
        assert!(!options.cancel.load(Ordering::Acquire));
        cancellation.cancel();
        assert!(options.cancel.load(Ordering::Acquire));
        let _: &AtomicBool = cancellation.flag();
    }

    #[test]
    fn provider_descriptors_use_distinct_stable_ids() {
        let descriptors = [
            ApplicationProvider::default().descriptor(),
            SettingsProvider::default().descriptor(),
            FileProvider::system(PathBuf::from("/home/alex")).descriptor(),
            CalculatorProvider.descriptor(),
        ];
        let unique: BTreeMap<_, _> = descriptors
            .iter()
            .map(|descriptor| (descriptor.id.clone(), descriptor.category))
            .collect();
        assert_eq!(unique.len(), descriptors.len());
    }
}
