//! Platform filename, recent-document, and tag search providers.

use super::*;

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
pub(super) fn spotlight(
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
        check_cancelled(options.cancel)?;
        let rmac_snapshot = rmac_recent_documents::Store::from_environment()
            .and_then(|store| store.load())
            .map_err(|_| Error::RecentDocuments)?;
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/share"))
            });
        let desktop_recents = match data_home {
            Some(data_home) => recent_from_path(
                &data_home.join("recently-used.xbel"),
                options,
                rmac_snapshot.cleared_before_unix_ms,
            )?,
            None => Vec::new(),
        };
        merge_recent_paths(rmac_snapshot.paths, desktop_recents, options)
    }

    fn tagged(&self, _tag: &str, _options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
        Err(Error::Unsupported("file tags"))
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn filesystem_search(
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
pub(super) fn recent_from_path(
    path: &Path,
    options: Options<'_>,
    modified_after_unix_ms: Option<u64>,
) -> Result<Vec<PathBuf>, Error> {
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
                let admitted_by_clear = modified_after_unix_ms.is_none_or(|boundary| {
                    recent_modified_unix_ms(&modified).is_some_and(|modified| modified > boundary)
                });
                if admitted_by_clear {
                    if let Some(path) = href
                        .and_then(|href| url::Url::parse(&href).ok())
                        .and_then(|url| url.to_file_path().ok())
                        .filter(|path| path.exists())
                        .filter(|path| !is_excluded(path, options.excluded_roots))
                    {
                        bookmarks.push((modified, path));
                    }
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

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn recent_modified_unix_ms(value: &str) -> Option<u64> {
    let milliseconds = chrono::DateTime::parse_from_rfc3339(value)
        .ok()?
        .timestamp_millis();
    u64::try_from(milliseconds).ok()
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn merge_recent_paths(
    rmac_recents: Vec<PathBuf>,
    desktop_recents: Vec<PathBuf>,
    options: Options<'_>,
) -> Result<Vec<PathBuf>, Error> {
    if options.limit == 0 {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for path in rmac_recents.into_iter().chain(desktop_recents) {
        check_cancelled(options.cancel)?;
        if !path.exists() || is_excluded(&path, options.excluded_roots) {
            continue;
        }
        let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.insert(identity) {
            merged.push(path);
            if merged.len() == options.limit {
                break;
            }
        }
    }
    Ok(merged)
}
