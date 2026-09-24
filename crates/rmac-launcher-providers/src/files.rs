//! Privacy-bounded file and recent-document launcher provider.

use super::*;

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
    let parent = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned());
    if !rmac_launcher::query_matches(query, &title, parent.as_deref()) {
        return None;
    }
    let local = path.to_string_lossy().into_owned();
    let subtitle = file_details(&path, &Locale::from_environment(), chrono::Local::now());
    Some(SearchResult {
        id: ResultId {
            provider: provider_id(FILES_PROVIDER),
            local,
        },
        category: Category::Files,
        application_group: None,
        title,
        subtitle,
        detail: None,
        icon: None,
        primary: Action::OpenFile { path: path.clone() },
        alternate: Some(Action::RevealFile { path }),
        recency_rank: 0,
    })
}

/// The file row's second line as macOS 26 writes it, less the kind rmac
/// has no authority for: "24 KB · Today, 9:08 PM · rmac" (the view draws a
/// folder glyph before the last part). Folders omit the size.
fn file_details(
    path: &Path,
    locale: &Locale,
    now: chrono::DateTime<chrono::Local>,
) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let size = metadata.is_file().then_some(metadata.len());
    let modified = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Local>::from);
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    let mut parts = Vec::new();
    if let Some(size) = size {
        parts.push(file_size(size));
    } else if metadata.is_dir() {
        parts.push("Folder".to_owned());
    }
    if let Some(modified) = modified {
        use chrono::{Datelike as _, Timelike as _};
        parts.push(file_date(
            (modified.year(), modified.month(), modified.day()),
            (modified.hour(), modified.minute()),
            (now.year(), now.month(), now.day()),
            now.date_naive()
                .pred_opt()
                .map(|yesterday| (yesterday.year(), yesterday.month(), yesterday.day())),
            locale,
        ));
    }
    parts.extend(folder);
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// "926 bytes", "24 KB", "1.3 MB", "2.15 GB": decimal units, as Finder.
pub fn file_size(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    let value = bytes as f64;
    if bytes == 1 {
        "1 byte".into()
    } else if bytes < 1_000 {
        format!("{bytes} bytes")
    } else if value < KB * KB {
        format!("{} KB", (value / KB).round().max(1.0))
    } else if value < KB * KB * KB {
        trim_decimals(format!("{:.1}", value / (KB * KB)), "MB")
    } else if value < KB.powi(4) {
        trim_decimals(format!("{:.2}", value / KB.powi(3)), "GB")
    } else {
        trim_decimals(format!("{:.2}", value / KB.powi(4)), "TB")
    }
}

fn trim_decimals(number: String, unit: &str) -> String {
    let number = number.trim_end_matches('0').trim_end_matches('.');
    format!("{number} {unit}")
}

/// "Today, 9:08 PM", "Yesterday, 8:20 PM" or "17/09/26, 3:33 PM".
pub fn file_date(
    date: (i32, u32, u32),
    time: (u32, u32),
    today: (i32, u32, u32),
    yesterday: Option<(i32, u32, u32)>,
    locale: &Locale,
) -> String {
    let day = if date == today {
        "Today".to_owned()
    } else if Some(date) == yesterday {
        "Yesterday".to_owned()
    } else {
        locale.short_date(date.0, date.1, date.2)
    };
    format!("{day}, {}", crate::locale::time_12h(time.0, time.1))
}
