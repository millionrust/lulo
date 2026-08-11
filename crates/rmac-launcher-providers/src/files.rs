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
        application_group: None,
        title,
        subtitle,
        icon: None,
        primary: Action::OpenFile { path: path.clone() },
        alternate: Some(Action::RevealFile { path }),
        recency_rank: 0,
    })
}
