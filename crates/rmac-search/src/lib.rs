//! Cross-platform file search and recent-document providers.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const DEFAULT_LIMIT: usize = 500;
const DEFAULT_MAX_ENTRIES: usize = 100_000;
const DEFAULT_MAX_CONTENT_FILE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 512;
const MAX_EXCERPT_CHARACTERS: usize = 240;

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

pub fn filenames(root: &Path, query: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.filenames(root, query, options)
}

/// Search names and bounded regular UTF-8 file content without following links.
///
/// This portable authority is intentionally independent of Spotlight so Files
/// can present exact match reasons and excerpts with identical safety bounds on
/// Linux and macOS. The legacy filename provider remains available to callers
/// such as Launcher that prefer the platform index.
pub fn ranked(root: &Path, query: &str, options: Options<'_>) -> Result<SearchReport, Error> {
    filesystem_ranked_search(root, query, options)
}

pub fn recents(options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.recents(options)
}

pub fn tagged(tag: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.tagged(tag, options)
}

#[cfg(target_os = "macos")]
impl SearchProvider for SystemSearchProvider {
    fn filenames(
        &self,
        root: &Path,
        query: &str,
        options: Options<'_>,
    ) -> Result<Vec<PathBuf>, Error> {
        spotlight(
            &["-onlyin", &root.to_string_lossy(), query],
            Some(root),
            options,
        )
    }

    fn recents(&self, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
        spotlight(&["kMDItemLastUsedDate >= $time.today(-30)"], None, options)
    }

    fn tagged(&self, tag: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
        let escaped = tag.replace('\\', "\\\\").replace('\'', "\\'");
        spotlight(
            &[&format!("kMDItemUserTags == '{escaped}'c")],
            None,
            options,
        )
    }
}

#[cfg(target_os = "macos")]
fn spotlight(
    arguments: &[&str],
    root: Option<&Path>,
    options: Options<'_>,
) -> Result<Vec<PathBuf>, Error> {
    check_cancelled(options.cancel)?;
    let output = Command::new("mdfind")
        .args(arguments)
        .output()
        .map_err(|source| Error::Io {
            operation: "start Spotlight",
            path: PathBuf::from("mdfind"),
            source,
        })?;
    check_cancelled(options.cancel)?;
    if !output.status.success() {
        return Err(Error::Command {
            program: "mdfind",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let root_device = root.and_then(device_for_path);
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| {
            path.exists()
                && !is_excluded(path, options.excluded_roots)
                && (!options.stay_on_filesystem
                    || root_device.is_none()
                    || device_for_path(path) == root_device)
        })
        .take(options.limit)
        .collect())
}

#[cfg(not(target_os = "macos"))]
impl SearchProvider for SystemSearchProvider {
    fn filenames(
        &self,
        root: &Path,
        query: &str,
        options: Options<'_>,
    ) -> Result<Vec<PathBuf>, Error> {
        filesystem_search(root, query, options)
    }

    fn recents(&self, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/share"))
            });
        let Some(data_home) = data_home else {
            return Ok(Vec::new());
        };
        recent_from_path(&data_home.join("recently-used.xbel"), options)
    }

    fn tagged(&self, _tag: &str, _options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
        Err(Error::Unsupported("file tags"))
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn filesystem_search(
    root: &Path,
    query: &str,
    options: Options<'_>,
) -> Result<Vec<PathBuf>, Error> {
    let root_metadata = root.metadata().map_err(|source| Error::Io {
        operation: "search",
        path: root.to_path_buf(),
        source,
    })?;
    let root_device = device(&root_metadata);
    let query = query.trim().to_lowercase();
    if query.is_empty() || options.limit == 0 {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if is_lexically_excluded(entry.path(), options.excluded_roots)
                || (!options.include_hidden && entry.file_name().to_string_lossy().starts_with('.'))
            {
                return false;
            }
            !entry.file_type().is_dir()
                || !options.stay_on_filesystem
                || root_device.is_none()
                || entry.metadata().ok().and_then(|metadata| device(&metadata)) == root_device
        });
    for result in walker {
        check_cancelled(options.cancel)?;
        let Ok(entry) = result else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        if entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .contains(&query)
        {
            paths.push(entry.into_path());
            if paths.len() == options.limit {
                break;
            }
        }
    }
    Ok(paths)
}

fn filesystem_ranked_search(
    root: &Path,
    query: &str,
    options: Options<'_>,
) -> Result<SearchReport, Error> {
    let root_metadata = root.metadata().map_err(|source| Error::Io {
        operation: "search",
        path: root.to_path_buf(),
        source,
    })?;
    let root_device = device(&root_metadata);
    let query = validate_query(query)?;
    if query.is_empty() || options.limit == 0 {
        return Ok(SearchReport::default());
    }

    let mut report = SearchReport::default();
    let mut exact = Vec::new();
    let mut prefixes = Vec::new();
    let mut substrings = Vec::new();
    let mut contents = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if is_lexically_excluded(entry.path(), options.excluded_roots)
                || (!options.include_hidden && entry.file_name().to_string_lossy().starts_with('.'))
            {
                return false;
            }
            !entry.file_type().is_dir()
                || !options.stay_on_filesystem
                || root_device.is_none()
                || std::fs::symlink_metadata(entry.path())
                    .ok()
                    .and_then(|metadata| device(&metadata))
                    == root_device
        });

    for result in walker {
        check_cancelled(options.cancel)?;
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => {
                report.skipped_errors = report.skipped_errors.saturating_add(1);
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        if report.scanned_entries == options.max_entries {
            report.entry_limit_reached = true;
            break;
        }
        report.scanned_entries += 1;

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let kind = if name == query {
            Some(MatchKind::ExactName)
        } else if name.starts_with(&query) {
            Some(MatchKind::NamePrefix)
        } else if name.contains(&query) {
            Some(MatchKind::NameSubstring)
        } else {
            None
        };

        if let Some(kind) = kind {
            let candidate = SearchMatch {
                path: path.to_path_buf(),
                kind,
                excerpt: None,
            };
            let bucket = match kind {
                MatchKind::ExactName => &mut exact,
                MatchKind::NamePrefix => &mut prefixes,
                MatchKind::NameSubstring => &mut substrings,
                MatchKind::Content => unreachable!(),
            };
            offer_match(bucket, candidate, options.limit, &mut report);
            continue;
        }

        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => continue,
            Err(_) => {
                report.skipped_errors = report.skipped_errors.saturating_add(1);
                continue;
            }
        };
        if let Some(excerpt) = content_excerpt(path, &metadata, &query, options, &mut report)? {
            offer_match(
                &mut contents,
                SearchMatch {
                    path: path.to_path_buf(),
                    kind: MatchKind::Content,
                    excerpt: Some(excerpt),
                },
                options.limit,
                &mut report,
            );
        }
    }

    for bucket in [&mut exact, &mut prefixes, &mut substrings, &mut contents] {
        bucket.sort_by(|left, right| match_path_key(&left.path).cmp(&match_path_key(&right.path)));
    }
    let candidate_count = exact
        .len()
        .saturating_add(prefixes.len())
        .saturating_add(substrings.len())
        .saturating_add(contents.len());
    report.results_truncated |= candidate_count > options.limit;
    report.matches = exact
        .into_iter()
        .chain(prefixes)
        .chain(substrings)
        .chain(contents)
        .take(options.limit)
        .collect();
    Ok(report)
}

fn validate_query(query: &str) -> Result<String, Error> {
    let query = query.trim();
    if query.len() > MAX_QUERY_BYTES {
        return Err(Error::InvalidQuery("query is too long"));
    }
    if query.chars().any(char::is_control) {
        return Err(Error::InvalidQuery("query contains a control character"));
    }
    Ok(query.to_lowercase())
}

fn offer_match(
    bucket: &mut Vec<SearchMatch>,
    candidate: SearchMatch,
    limit: usize,
    report: &mut SearchReport,
) {
    if bucket.len() < limit {
        bucket.push(candidate);
    } else {
        report.results_truncated = true;
    }
}

fn match_path_key(path: &Path) -> (String, String) {
    (
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase(),
        path.to_string_lossy().to_lowercase(),
    )
}

fn content_excerpt(
    path: &Path,
    expected: &std::fs::Metadata,
    query: &str,
    options: Options<'_>,
    report: &mut SearchReport,
) -> Result<Option<String>, Error> {
    let remaining = options
        .max_total_content_bytes
        .saturating_sub(report.content_bytes);
    let expected_len = usize::try_from(expected.len()).unwrap_or(usize::MAX);
    let read_limit = remaining
        .min(options.max_content_file_bytes)
        .min(expected_len);
    if read_limit < expected_len {
        report.content_partially_scanned = true;
    }
    if read_limit == 0 {
        return Ok(None);
    }

    let mut file = match open_regular_no_follow(path) {
        Ok(file) => file,
        Err(_) => {
            report.skipped_errors = report.skipped_errors.saturating_add(1);
            return Ok(None);
        }
    };
    let opened = match file.metadata() {
        Ok(metadata) if metadata.is_file() && same_content_identity(expected, &metadata) => {
            metadata
        }
        _ => {
            report.skipped_errors = report.skipped_errors.saturating_add(1);
            return Ok(None);
        }
    };

    let mut bytes = Vec::with_capacity(read_limit.min(64 * 1024));
    let mut chunk = [0_u8; 8192];
    while bytes.len() < read_limit {
        check_cancelled(options.cancel)?;
        let wanted = chunk.len().min(read_limit - bytes.len());
        match file.read(&mut chunk[..wanted]) {
            Ok(0) => break,
            Ok(read) => {
                report.content_bytes = report.content_bytes.saturating_add(read);
                bytes.extend_from_slice(&chunk[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                report.skipped_errors = report.skipped_errors.saturating_add(1);
                return Ok(None);
            }
        }
    }
    check_cancelled(options.cancel)?;

    let current_handle = file.metadata().ok();
    let current_path = std::fs::symlink_metadata(path).ok();
    if current_handle
        .as_ref()
        .is_none_or(|metadata| !same_content_identity(&opened, metadata))
        || current_path
            .as_ref()
            .is_none_or(|metadata| !same_content_identity(&opened, metadata))
    {
        report.skipped_errors = report.skipped_errors.saturating_add(1);
        return Ok(None);
    }

    let text = match bounded_utf8(&bytes) {
        Some(text) if !looks_binary(text) => text,
        _ => return Ok(None),
    };
    Ok(text.lines().find_map(|line| {
        line.to_lowercase()
            .contains(query)
            .then(|| bounded_excerpt(line))
    }))
}

#[cfg(unix)]
fn open_regular_no_follow(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(not(unix))]
fn open_regular_no_follow(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).open(path)
}

fn bounded_utf8(bytes: &[u8]) -> Option<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

fn looks_binary(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn bounded_excerpt(line: &str) -> String {
    let mut excerpt = String::new();
    let mut characters = 0;
    let mut pending_space = false;
    let mut truncated = false;
    for character in line.trim().chars() {
        if character.is_whitespace() {
            pending_space = !excerpt.is_empty();
            continue;
        }
        if pending_space {
            if characters == MAX_EXCERPT_CHARACTERS {
                truncated = true;
                break;
            }
            excerpt.push(' ');
            characters += 1;
            pending_space = false;
        }
        if characters == MAX_EXCERPT_CHARACTERS {
            truncated = true;
            break;
        }
        excerpt.push(character);
        characters += 1;
    }
    if truncated {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(unix)]
fn same_content_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(not(unix))]
fn same_content_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.len() == right.len()
        && left.is_file() == right.is_file()
        && left.modified().ok() == right.modified().ok()
}

#[cfg(any(not(target_os = "macos"), test))]
fn recent_from_path(path: &Path, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    use quick_xml::events::Event;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                operation: "open recent documents",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut reader = quick_xml::Reader::from_reader(io::BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut bookmarks = Vec::new();
    loop {
        check_cancelled(options.cancel)?;
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element) | Event::Empty(element))
                if element.name().as_ref() == b"bookmark" =>
            {
                let mut href = None;
                let mut modified = String::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|error| Error::InvalidData {
                        path: path.to_path_buf(),
                        message: error.to_string(),
                    })?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|error| Error::InvalidData {
                            path: path.to_path_buf(),
                            message: error.to_string(),
                        })?
                        .into_owned();
                    match attribute.key.as_ref() {
                        b"href" => href = Some(value),
                        b"modified" => modified = value,
                        _ => {}
                    }
                }
                if let Some(path) = href
                    .and_then(|href| url::Url::parse(&href).ok())
                    .and_then(|url| url.to_file_path().ok())
                    .filter(|path| path.exists())
                    .filter(|path| !is_excluded(path, options.excluded_roots))
                {
                    bookmarks.push((modified, path));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(Error::InvalidData {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                });
            }
        }
        buffer.clear();
    }

    bookmarks.sort_by(|left, right| right.0.cmp(&left.0));
    let mut seen = HashSet::new();
    Ok(bookmarks
        .into_iter()
        .map(|(_, path)| path)
        .filter(|path| seen.insert(path.clone()))
        .take(options.limit)
        .collect())
}

fn is_excluded(path: &Path, excluded_roots: &[PathBuf]) -> bool {
    if is_lexically_excluded(path, excluded_roots) {
        return true;
    }
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    excluded_roots.iter().any(|excluded| {
        excluded
            .canonicalize()
            .is_ok_and(|excluded| canonical.starts_with(excluded))
    })
}

fn is_lexically_excluded(path: &Path, excluded_roots: &[PathBuf]) -> bool {
    excluded_roots
        .iter()
        .any(|excluded| path.starts_with(excluded))
}

#[cfg(unix)]
fn device(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;

    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device(_: &std::fs::Metadata) -> Option<u64> {
    None
}

fn device_for_path(path: &Path) -> Option<u64> {
    path.metadata().ok().and_then(|metadata| device(&metadata))
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn filesystem_search_filters_hidden_entries_and_honors_cancellation() {
        let root = temporary_directory("filenames");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("nested/Quarterly Report.txt"), b"report").unwrap();
        std::fs::write(root.join(".hidden/secret-report.txt"), b"secret").unwrap();
        let cancel = AtomicBool::new(false);

        let paths = filesystem_search(&root, "REPORT", Options::new(&cancel)).unwrap();
        assert_eq!(paths, [root.join("nested/Quarterly Report.txt")]);

        let mut limited = Options::new(&cancel);
        limited.include_hidden = true;
        limited.limit = 1;
        assert_eq!(
            filesystem_search(&root, "report", limited).unwrap().len(),
            1
        );

        cancel.store(true, Ordering::Release);
        assert!(matches!(
            filesystem_search(&root, "report", Options::new(&cancel)),
            Err(Error::Cancelled)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ranked_search_orders_name_relevance_before_content_and_explains_content() {
        let root = temporary_directory("ranked");
        std::fs::create_dir_all(&root).unwrap();
        let exact = root.join("NEEDLE");
        let prefix = root.join("needle planning.txt");
        let substring = root.join("a-needle-copy.txt");
        let content = root.join("meeting notes.txt");
        std::fs::write(&exact, b"nothing").unwrap();
        std::fs::write(&prefix, b"nothing").unwrap();
        std::fs::write(&substring, b"nothing").unwrap();
        std::fs::write(
            &content,
            b"First line\n  The requested NEEDLE is in this line.  \nLast line",
        )
        .unwrap();
        let cancel = AtomicBool::new(false);

        let report = filesystem_ranked_search(&root, "needle", Options::new(&cancel)).unwrap();

        assert_eq!(
            report
                .matches
                .iter()
                .map(|result| (&result.path, result.kind))
                .collect::<Vec<_>>(),
            [
                (&exact, MatchKind::ExactName),
                (&prefix, MatchKind::NamePrefix),
                (&substring, MatchKind::NameSubstring),
                (&content, MatchKind::Content),
            ]
        );
        assert_eq!(
            report.matches[3].excerpt.as_deref(),
            Some("The requested NEEDLE is in this line.")
        );
        assert!(!report.results_truncated);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ranked_search_reports_result_traversal_and_content_bounds() {
        let root = temporary_directory("ranked-bounds");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("needle-a"), b"none").unwrap();
        std::fs::write(root.join("needle-b"), b"none").unwrap();
        std::fs::write(root.join("needle-c"), b"none").unwrap();
        let cancel = AtomicBool::new(false);
        let mut limited = Options::new(&cancel);
        limited.limit = 2;

        let report = filesystem_ranked_search(&root, "needle", limited).unwrap();
        assert_eq!(report.matches.len(), 2);
        assert!(report.results_truncated);

        let mut traversal_limited = Options::new(&cancel);
        traversal_limited.max_entries = 1;
        let report = filesystem_ranked_search(&root, "absent query", traversal_limited).unwrap();
        assert_eq!(report.scanned_entries, 1);
        assert!(report.entry_limit_reached);

        let content_root = temporary_directory("ranked-content-bound");
        std::fs::create_dir_all(&content_root).unwrap();
        std::fs::write(content_root.join("body.txt"), b"prefix needle suffix").unwrap();
        let mut content_limited = Options::new(&cancel);
        content_limited.max_content_file_bytes = 6;
        content_limited.max_total_content_bytes = 6;
        let report = filesystem_ranked_search(&content_root, "needle", content_limited).unwrap();
        assert!(report.matches.is_empty());
        assert_eq!(report.content_bytes, 6);
        assert!(report.content_partially_scanned);

        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(content_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ranked_content_search_prunes_hidden_excluded_and_symbolic_link_inputs() {
        use std::os::unix::fs::symlink;

        let root = temporary_directory("ranked-safety");
        let hidden = root.join(".hidden");
        let excluded = root.join("Private");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::create_dir_all(&excluded).unwrap();
        let visible = root.join("visible.txt");
        std::fs::write(&visible, b"contains private phrase").unwrap();
        std::fs::write(hidden.join("hidden.txt"), b"contains private phrase").unwrap();
        std::fs::write(excluded.join("excluded.txt"), b"contains private phrase").unwrap();
        symlink(&visible, root.join("alias.txt")).unwrap();
        let cancel = AtomicBool::new(false);
        let exclusions = vec![excluded];
        let mut options = Options::new(&cancel);
        options.excluded_roots = &exclusions;

        let report = filesystem_ranked_search(&root, "private phrase", options).unwrap();

        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].path, visible);
        assert_eq!(report.matches[0].kind, MatchKind::Content);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ranked_search_rejects_unbounded_or_control_queries_and_cancels() {
        let root = temporary_directory("ranked-query");
        std::fs::create_dir_all(&root).unwrap();
        let cancel = AtomicBool::new(false);

        assert!(matches!(
            filesystem_ranked_search(
                &root,
                &"x".repeat(MAX_QUERY_BYTES + 1),
                Options::new(&cancel)
            ),
            Err(Error::InvalidQuery(_))
        ));
        assert!(matches!(
            filesystem_ranked_search(&root, "line\nbreak", Options::new(&cancel)),
            Err(Error::InvalidQuery(_))
        ));
        cancel.store(true, Ordering::Release);
        assert!(matches!(
            filesystem_ranked_search(&root, "anything", Options::new(&cancel)),
            Err(Error::Cancelled)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_xbel_keeps_existing_file_urls_in_modified_order() {
        let root = temporary_directory("recents");
        std::fs::create_dir_all(&root).unwrap();
        let older = root.join("older file.txt");
        let newer = root.join("newer.txt");
        std::fs::write(&older, b"older").unwrap();
        std::fs::write(&newer, b"newer").unwrap();
        let older_url = url::Url::from_file_path(&older).unwrap();
        let newer_url = url::Url::from_file_path(&newer).unwrap();
        let xbel = root.join("recently-used.xbel");
        std::fs::write(
            &xbel,
            format!(
                "<?xml version=\"1.0\"?><xbel version=\"1.0\">\
                 <bookmark href=\"{older_url}\" modified=\"2026-01-01T00:00:00Z\"/>\
                 <bookmark href=\"https://example.com\" modified=\"2026-03-01T00:00:00Z\"/>\
                 <bookmark href=\"{newer_url}\" modified=\"2026-02-01T00:00:00Z\"/>\
                 </xbel>"
            ),
        )
        .unwrap();
        let cancel = AtomicBool::new(false);

        let paths = recent_from_path(&xbel, Options::new(&cancel)).unwrap();
        assert_eq!(paths, [newer, older]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filename_and_recent_searches_prune_excluded_roots_and_stale_records() {
        let root = temporary_directory("exclusions");
        let included = root.join("Documents");
        let excluded = root.join("Private");
        std::fs::create_dir_all(&included).unwrap();
        std::fs::create_dir_all(&excluded).unwrap();
        let visible = included.join("Visible Report.txt");
        let secret = excluded.join("Secret Report.txt");
        let stale = root.join("Deleted Report.txt");
        std::fs::write(&visible, b"visible").unwrap();
        std::fs::write(&secret, b"secret").unwrap();
        let cancel = AtomicBool::new(false);
        let exclusions = vec![excluded.clone()];
        let mut options = Options::new(&cancel);
        options.excluded_roots = &exclusions;

        assert_eq!(
            filesystem_search(&root, "report", options).unwrap(),
            std::slice::from_ref(&visible)
        );

        let xbel = root.join("recently-used.xbel");
        let visible_url = url::Url::from_file_path(&visible).unwrap();
        let secret_url = url::Url::from_file_path(&secret).unwrap();
        let stale_url = url::Url::from_file_path(&stale).unwrap();
        std::fs::write(
            &xbel,
            format!(
                "<?xml version=\"1.0\"?><xbel version=\"1.0\">\
                 <bookmark href=\"{secret_url}\" modified=\"2026-03-01T00:00:00Z\"/>\
                 <bookmark href=\"{stale_url}\" modified=\"2026-02-01T00:00:00Z\"/>\
                 <bookmark href=\"{visible_url}\" modified=\"2026-01-01T00:00:00Z\"/>\
                 </xbel>"
            ),
        )
        .unwrap();
        assert_eq!(recent_from_path(&xbel, options).unwrap(), [visible]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_exclusions_cover_recent_paths_reached_through_symlinks() {
        use std::os::unix::fs::symlink;

        let root = temporary_directory("symlink-exclusion");
        let excluded = root.join("Private");
        let alias = root.join("Alias");
        std::fs::create_dir_all(&excluded).unwrap();
        let secret = excluded.join("Secret.txt");
        std::fs::write(&secret, b"secret").unwrap();
        symlink(&excluded, &alias).unwrap();
        let aliased_secret = alias.join("Secret.txt");
        assert!(aliased_secret.exists());
        assert!(is_excluded(
            &aliased_secret,
            std::slice::from_ref(&excluded)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_xbel_distinguishes_missing_and_malformed_stores() {
        let root = temporary_directory("recent-errors");
        std::fs::create_dir_all(&root).unwrap();
        let missing = root.join("missing.xbel");
        let malformed = root.join("malformed.xbel");
        std::fs::write(&malformed, "<xbel><bookmark").unwrap();
        let cancel = AtomicBool::new(false);

        assert!(recent_from_path(&missing, Options::new(&cancel))
            .unwrap()
            .is_empty());
        assert!(matches!(
            recent_from_path(&malformed, Options::new(&cancel)),
            Err(Error::InvalidData { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-search-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
