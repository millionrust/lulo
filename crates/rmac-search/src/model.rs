//! Stable search options, results, errors, and provider contract.

use super::*;

#[derive(Debug)]
pub enum Error {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Command {
        program: &'static str,
        message: String,
    },
    InvalidData {
        path: PathBuf,
        message: String,
    },
    InvalidQuery(&'static str),
    Cancelled,
    RecentDocuments,
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Command { program, message } => {
                write!(formatter, "{program} search failed: {message}")
            }
            Self::InvalidData { path, message } => {
                write!(formatter, "could not read {}: {message}", path.display())
            }
            Self::InvalidQuery(message) => write!(formatter, "invalid search: {message}"),
            Self::Cancelled => formatter.write_str("search cancelled"),
            Self::RecentDocuments => {
                formatter.write_str("recent documents are temporarily unavailable")
            }
            Self::Unsupported(capability) => {
                write!(formatter, "{capability} is not supported on this platform")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub include_hidden: bool,
    pub limit: usize,
    pub cancel: &'a AtomicBool,
    pub excluded_roots: &'a [PathBuf],
    pub stay_on_filesystem: bool,
    /// Maximum number of directory entries inspected by a ranked search.
    pub max_entries: usize,
    /// Maximum prefix inspected from any one regular UTF-8 file.
    pub max_content_file_bytes: usize,
    /// Maximum aggregate file content inspected by one ranked search.
    pub max_total_content_bytes: usize,
}

impl<'a> Options<'a> {
    pub fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            include_hidden: false,
            limit: DEFAULT_LIMIT,
            cancel,
            excluded_roots: &[],
            stay_on_filesystem: true,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_content_file_bytes: DEFAULT_MAX_CONTENT_FILE_BYTES,
            max_total_content_bytes: DEFAULT_MAX_TOTAL_CONTENT_BYTES,
        }
    }
}

/// Why a path matched, in descending relevance order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    ExactName,
    NamePrefix,
    NameSubstring,
    Content,
}

/// One bounded, identity-verified ranked search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub kind: MatchKind,
    /// A single sanitized, bounded line for a content match.
    pub excerpt: Option<String>,
}

/// Results and truthful scope information for a ranked filesystem search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchReport {
    pub matches: Vec<SearchMatch>,
    pub scanned_entries: usize,
    pub content_bytes: usize,
    pub results_truncated: bool,
    pub entry_limit_reached: bool,
    pub content_partially_scanned: bool,
    pub skipped_errors: usize,
}

pub trait SearchProvider {
    fn filenames(
        &self,
        root: &Path,
        query: &str,
        options: Options<'_>,
    ) -> Result<Vec<PathBuf>, Error>;

    fn recents(&self, options: Options<'_>) -> Result<Vec<PathBuf>, Error>;

    fn tagged(&self, tag: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error>;
}

pub struct SystemSearchProvider;
