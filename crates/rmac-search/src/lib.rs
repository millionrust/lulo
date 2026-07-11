//! Cross-platform file search and recent-document providers.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const DEFAULT_LIMIT: usize = 500;

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
}

impl<'a> Options<'a> {
    pub fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            include_hidden: false,
            limit: DEFAULT_LIMIT,
            cancel,
            excluded_roots: &[],
            stay_on_filesystem: true,
        }
    }
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
            [visible.clone()]
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
