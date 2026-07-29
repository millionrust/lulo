//! Local application, setting, file, and calculator launcher providers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

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

    pub fn system_settings() -> Self {
        Self::new(system_settings_entries())
    }
}

pub fn system_settings_entries() -> Vec<SettingEntry> {
    [
        (
            "wifi",
            "Wi-Fi",
            "Wireless network connections",
            &["wireless", "wlan", "internet"][..],
        ),
        (
            "bluetooth",
            "Bluetooth",
            "Nearby devices and accessories",
            &["headphones", "keyboard", "pair device"],
        ),
        (
            "network",
            "Network",
            "Ethernet, DNS, and proxy settings",
            &["ethernet", "dns", "proxy", "connection"],
        ),
        (
            "vpn",
            "VPN",
            "Private network connections",
            &["tunnel", "work network"],
        ),
        (
            "battery",
            "Battery",
            "Energy use and power behavior",
            &["power", "energy", "charging"],
        ),
        (
            "general",
            "General",
            "System information, updates, and storage",
            &["about", "update", "storage", "system information", "backup"],
        ),
        (
            "date-time",
            "Date & Time",
            "Time zone and automatic clock settings",
            &["clock", "timezone", "ntp", "automatic time"],
        ),
        (
            "language-region",
            "Language & Region",
            "Language, formats, and keyboard layouts",
            &["locale", "formats", "region", "xkb", "input source"],
        ),
        (
            "login-items",
            "Login Items",
            "Applications and services that start at sign in",
            &["startup", "autostart", "systemd user"],
        ),
        (
            "sharing",
            "Sharing",
            "Remote login and file sharing",
            &["ssh", "samba", "remote access", "shared folders"],
        ),
        (
            "accessibility",
            "Accessibility",
            "Vision, hearing, motor, and speech support",
            &[
                "screen reader",
                "zoom",
                "contrast",
                "reduce motion",
                "assistive",
            ],
        ),
        (
            "appearance",
            "Appearance",
            "Light, dark, accent, and interface style",
            &["theme", "dark mode", "light mode", "accent", "color"],
        ),
        (
            "desktop-dock",
            "Desktop & Dock",
            "Dock, windows, workspaces, and desktop behavior",
            &["dock", "windows", "workspace", "autohide", "magnification"],
        ),
        (
            "displays",
            "Displays",
            "Resolution, scale, arrangement, and brightness",
            &["monitor", "screen", "resolution", "scaling", "brightness"],
        ),
        (
            "spotlight",
            "Spotlight",
            "Search providers, privacy, exclusions, and shortcut",
            &[
                "search", "launcher", "indexing", "privacy", "exclude", "shortcut",
            ],
        ),
        (
            "wallpaper",
            "Wallpaper",
            "Desktop background for each display",
            &["background", "desktop picture", "image"],
        ),
        (
            "notifications",
            "Notifications",
            "Alerts, banners, and application policy",
            &["alerts", "banners", "notification center"],
        ),
        (
            "sound",
            "Sound",
            "Output, input, effects, and volume",
            &["volume", "speaker", "microphone", "audio", "mute"],
        ),
        (
            "keyboard",
            "Keyboard",
            "Key repeat, input, and shortcuts",
            &["keys", "repeat", "input source", "shortcut"],
        ),
        (
            "mouse",
            "Mouse",
            "Pointer, scrolling, acceleration, and buttons",
            &["pointer", "scroll", "click", "acceleration"],
        ),
        (
            "trackpad",
            "Trackpad",
            "Tracking, tapping, scrolling, and gestures",
            &["touchpad", "gesture", "tap", "scroll"],
        ),
        (
            "focus",
            "Focus",
            "Silence interruptions with Focus modes",
            &["do not disturb", "quiet", "notifications"],
        ),
        (
            "lock-screen",
            "Lock Screen",
            "Lock, login, and idle timeout behavior",
            &["lock", "login", "password", "timeout", "idle"],
        ),
        (
            "privacy-security",
            "Privacy & Security",
            "Permissions, firewall, and system security",
            &[
                "permissions",
                "firewall",
                "encryption",
                "security",
                "privacy",
            ],
        ),
    ]
    .into_iter()
    .map(|(pane_id, title, subtitle, keywords)| SettingEntry {
        pane_id: pane_id.into(),
        title: title.into(),
        subtitle: Some(subtitle.into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
    })
    .collect()
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
                icon: None,
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
    excluded_roots: Vec<PathBuf>,
    include_removable_mounts: bool,
    search: S,
}

impl FileProvider<SystemFileSearch> {
    pub fn system(root: PathBuf) -> Self {
        Self {
            root,
            excluded_roots: Vec::new(),
            include_removable_mounts: false,
            search: SystemFileSearch,
        }
    }
}

impl<S> FileProvider<S> {
    pub fn new(root: PathBuf, search: S) -> Self {
        Self {
            root,
            excluded_roots: Vec::new(),
            include_removable_mounts: false,
            search,
        }
    }

    pub fn scoped(
        root: PathBuf,
        settings: &rmac_shell_settings::SpotlightSettings,
        search: S,
    ) -> Result<Self, ProviderError> {
        if !root.is_absolute() {
            return Err(ProviderError {
                detail: "file search root must be absolute".into(),
            });
        }
        let mut seen = BTreeSet::new();
        let mut excluded_roots = Vec::new();
        for excluded in &settings.excluded_paths {
            let path = PathBuf::from(excluded);
            if !path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
            {
                return Err(ProviderError {
                    detail: "file exclusions must be normalized absolute paths".into(),
                });
            }
            if seen.insert(path.clone()) {
                excluded_roots.push(path);
            }
        }
        Ok(Self {
            root,
            excluded_roots,
            include_removable_mounts: settings.include_removable_mounts,
            search,
        })
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
        options.excluded_roots = &self.excluded_roots;
        options.stay_on_filesystem = !self.include_removable_mounts;
        let paths = if query.trim().is_empty() {
            self.search.recents(options)
        } else {
            self.search.filenames(&self.root, query, options)
        }
        .map_err(|error| ProviderError {
            detail: error.to_string(),
        })?;
        let mut seen = BTreeSet::new();
        let mut results = Vec::new();
        for path in paths {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            if !self.path_allowed(&path, query) || !seen.insert(path.clone()) {
                continue;
            }
            if let Some(result) = file_result(path, query) {
                results.push(result);
                if results.len() == PROVIDER_LIMIT {
                    break;
                }
            }
        }
        Ok(results)
    }
}

impl<S> FileProvider<S> {
    fn path_allowed(&self, path: &Path, query: &str) -> bool {
        path.is_absolute()
            && path.exists()
            && !self
                .excluded_roots
                .iter()
                .any(|excluded| excluded_path(path, excluded))
            && (path.starts_with(&self.root)
                || (query.trim().is_empty() && self.include_removable_mounts))
            && (self.include_removable_mounts || same_filesystem(&self.root, path))
    }
}

fn excluded_path(path: &Path, excluded: &Path) -> bool {
    path.starts_with(excluded)
        || path.canonicalize().is_ok_and(|path| {
            excluded
                .canonicalize()
                .is_ok_and(|excluded| path.starts_with(excluded))
        })
}

#[cfg(unix)]
fn same_filesystem(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.metadata()
        .and_then(|left| right.metadata().map(|right| left.dev() == right.dev()))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn same_filesystem(_: &Path, _: &Path) -> bool {
    true
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
        icon: None,
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
            icon: None,
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: vec!["Utility".into()],
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.into(),
                args: vec!["--new".into()],
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn application_provider_preserves_exact_launch_spec() {
        let mut terminal = application("terminal.desktop", "Terminal");
        terminal.generic_name = Some("Console".into());
        terminal.keywords = vec!["shell".into(), "command line".into()];
        terminal.actions.push(rmac_apps::DesktopAction {
            id: "New-Window".into(),
            name: "Fresh Window".into(),
            icon: None,
            launch: terminal.launch.clone(),
        });
        let provider = ApplicationProvider::new(vec![terminal]);
        let results = provider
            .search("term", &Cancellation::default())
            .expect("search succeeds");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            &results[0].primary,
            Action::LaunchApplication { app_id, spec: rmac_apps::LaunchSpec::Command { args, .. } }
                if app_id == "terminal.desktop" && args == &["--new"]
        ));
        assert!(matches!(
            &results[0].alternate,
            Some(Action::RevealApplication { source })
                if source == Path::new("/apps/terminal.desktop")
        ));
        for metadata_query in ["console", "shell", "fresh window"] {
            assert_eq!(
                provider
                    .search(metadata_query, &Cancellation::default())
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn application_catalog_replacement_is_atomic_shared_and_revision_stable() {
        let provider = ApplicationProvider::new(vec![application("terminal.desktop", "Terminal")]);
        let clone = provider.clone();
        let revision = provider.revision();
        assert!(revision > 0);
        assert!(!provider.replace_catalog(vec![application("terminal.desktop", "Terminal")]));
        assert_eq!(provider.revision(), revision);

        assert!(provider.replace_catalog(vec![
            application("notes.desktop", "Notes"),
            application("notes.desktop", "Duplicate Notes"),
            application("", "Invalid"),
        ]));
        assert!(provider.revision() > revision);
        let results = clone
            .search("", &Cancellation::default())
            .expect("shared catalog search succeeds");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.local, "notes.desktop");
        assert_eq!(results[0].title, "Notes");
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

    #[test]
    fn system_settings_catalog_has_stable_unique_panes_and_linux_synonyms() {
        let entries = system_settings_entries();
        assert_eq!(entries.len(), 24);
        let unique: BTreeSet<_> = entries.iter().map(|entry| &entry.pane_id).collect();
        assert_eq!(unique.len(), entries.len());
        let provider = SettingsProvider::system_settings();
        let touchpad = provider
            .search("touchpad", &Cancellation::default())
            .expect("settings search succeeds");
        assert_eq!(touchpad[0].id.local, "trackpad");
        let firewall = provider
            .search("firewall", &Cancellation::default())
            .expect("settings search succeeds");
        assert_eq!(firewall[0].id.local, "privacy-security");
        assert!(entries.iter().all(|entry| entry.pane_id != "assistant"));
        assert!(entries.iter().all(|entry| entry.pane_id != "screen-time"));
        assert!(entries.iter().all(|entry| !entry.pane_id.contains(' ')));
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

        fn recents(
            &self,
            options: rmac_search::Options<'_>,
        ) -> Result<Vec<PathBuf>, rmac_search::Error> {
            if options.cancel.load(Ordering::Acquire) {
                return Err(rmac_search::Error::Cancelled);
            }
            Ok(self.paths.clone())
        }
    }

    #[test]
    fn file_provider_is_private_scoped_deduplicated_and_revealable() {
        let root = temporary_directory("scope");
        let excluded = root.join("Private");
        std::fs::create_dir_all(&excluded).expect("create test directories");
        let report = root.join("Report.txt");
        let secret = excluded.join("Secret Report.txt");
        std::fs::write(&report, b"report").expect("write report");
        std::fs::write(&secret, b"secret").expect("write secret");
        let settings = rmac_shell_settings::SpotlightSettings {
            excluded_paths: vec![excluded.to_string_lossy().into_owned()],
            include_removable_mounts: false,
        };
        let provider = FileProvider::scoped(
            root.clone(),
            &settings,
            FakeFileSearch {
                paths: vec![
                    report.clone(),
                    report,
                    secret,
                    root.join("stale-report.txt"),
                    PathBuf::from("relative.txt"),
                ],
            },
        )
        .expect("scope is valid");
        assert!(provider.descriptor().privacy.private_content);
        let results = provider
            .search("report", &Cancellation::default())
            .expect("search succeeds");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].alternate,
            Some(Action::RevealFile { .. })
        ));
        std::fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn recent_documents_outside_the_root_require_removable_mount_opt_in() {
        let root = temporary_directory("home");
        let external = temporary_directory("external");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&external).expect("create external root");
        let document = external.join("External.txt");
        std::fs::write(&document, b"external").expect("write external file");

        let default_provider = FileProvider::new(
            root.clone(),
            FakeFileSearch {
                paths: vec![document.clone()],
            },
        );
        assert!(default_provider
            .search("", &Cancellation::default())
            .expect("recent search succeeds")
            .is_empty());

        let opted_in = FileProvider::scoped(
            root.clone(),
            &rmac_shell_settings::SpotlightSettings {
                excluded_paths: Vec::new(),
                include_removable_mounts: true,
            },
            FakeFileSearch {
                paths: vec![document],
            },
        )
        .expect("scope is valid");
        assert_eq!(
            opted_in
                .search("", &Cancellation::default())
                .expect("recent search succeeds")
                .len(),
            1
        );
        std::fs::remove_dir_all(root).expect("remove root");
        std::fs::remove_dir_all(external).expect("remove external root");
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

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-launcher-providers-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock follows epoch")
                .as_nanos()
        ))
    }
}
