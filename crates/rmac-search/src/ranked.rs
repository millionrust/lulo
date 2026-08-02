//! Bounded ranked name and UTF-8 content search authority.

use super::*;

pub(super) fn filesystem_ranked_search(
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

pub(super) fn validate_query(query: &str) -> Result<String, Error> {
    let query = query.trim();
    if query.len() > MAX_QUERY_BYTES {
        return Err(Error::InvalidQuery("query is too long"));
    }
    if query.chars().any(char::is_control) {
        return Err(Error::InvalidQuery("query contains a control character"));
    }
    Ok(query.to_lowercase())
}

pub(super) fn offer_match(
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

pub(super) fn match_path_key(path: &Path) -> (String, String) {
    (
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase(),
        path.to_string_lossy().to_lowercase(),
    )
}

pub(super) fn content_excerpt(
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
pub(super) fn open_regular_no_follow(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(not(unix))]
pub(super) fn open_regular_no_follow(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).open(path)
}

pub(super) fn bounded_utf8(bytes: &[u8]) -> Option<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

pub(super) fn looks_binary(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

pub(super) fn bounded_excerpt(line: &str) -> String {
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
pub(super) fn same_content_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
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
pub(super) fn same_content_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.len() == right.len()
        && left.is_file() == right.is_file()
        && left.modified().ok() == right.modified().ok()
}

pub(super) fn is_excluded(path: &Path, excluded_roots: &[PathBuf]) -> bool {
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

pub(super) fn is_lexically_excluded(path: &Path, excluded_roots: &[PathBuf]) -> bool {
    excluded_roots
        .iter()
        .any(|excluded| path.starts_with(excluded))
}

#[cfg(unix)]
pub(super) fn device(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;

    Some(metadata.dev())
}

#[cfg(not(unix))]
pub(super) fn device(_: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub(super) fn device_for_path(path: &Path) -> Option<u64> {
    path.metadata().ok().and_then(|metadata| device(&metadata))
}

pub(super) fn check_cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
