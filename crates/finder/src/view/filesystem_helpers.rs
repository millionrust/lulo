use super::*;

pub(super) fn ranked_search_summary(
    report: &rmac_search::SearchReport,
    visible_count: usize,
) -> String {
    let mut summary = format!(
        "{visible_count} result{} · {} items checked",
        if visible_count == 1 { "" } else { "s" },
        report.scanned_entries
    );
    if report.results_truncated {
        summary.push_str(" · result limit reached");
    }
    if report.entry_limit_reached {
        summary.push_str(" · item limit reached");
    }
    if report.content_partially_scanned {
        summary.push_str(" · some content not searched");
    }
    if report.skipped_errors > 0 {
        summary.push_str(&format!(" · {} unavailable", report.skipped_errors));
    }
    summary
}

pub(super) fn ranked_search_error_message(error: &rmac_search::Error) -> String {
    match error {
        rmac_search::Error::InvalidQuery(_) => error.to_string(),
        _ => "Search could not safely read this folder".to_string(),
    }
}

pub(super) fn quick_look_error_message(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "This item is no longer available.",
        std::io::ErrorKind::PermissionDenied => {
            "Files does not have permission to preview this item."
        }
        std::io::ErrorKind::WouldBlock => "This item changed while its preview was loading.",
        std::io::ErrorKind::Interrupted => "Preview loading was cancelled.",
        _ => "Files could not safely render a preview for this item.",
    }
}

pub(super) fn search_entry_for(
    root: &Path,
    search_match: rmac_search::SearchMatch,
) -> Option<Entry> {
    std::fs::symlink_metadata(&search_match.path).ok()?;
    let mut entry = entry_for(&search_match.path)?;
    let detail = match search_match.kind {
        rmac_search::MatchKind::ExactName => {
            format!(
                "Exact name · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::NamePrefix => {
            format!(
                "Name begins with · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::NameSubstring => {
            format!(
                "Name contains · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::Content => format!(
            "Contents · {}",
            sanitize_dialog_name(search_match.excerpt.as_deref().unwrap_or("Matching text"))
        ),
    };
    entry.search_detail = Some(detail.into());
    Some(entry)
}

fn search_parent_label(root: &Path, path: &Path) -> String {
    let parent = path.parent().unwrap_or(root);
    match parent.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => "This folder".to_string(),
        Ok(relative) => sanitize_dialog_name(&relative.to_string_lossy()),
        Err(_) => "Current search scope".to_string(),
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn permanent_delete_prompt(count: usize, name: Option<&str>) -> String {
    if count == 1 {
        format!(
            "“{}” will be deleted immediately. This action cannot be undone. Deletion of an item cannot be cancelled once it begins.",
            name.unwrap_or("This item")
        )
    } else {
        format!(
            "{count} items will be deleted immediately. This action cannot be undone. Deletion of an item cannot be cancelled once it begins."
        )
    }
}

pub(super) fn unique_path(path: PathBuf) -> PathBuf {
    unique_path_avoiding(path, &BTreeSet::new())
}

pub(super) fn entry_for(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let md = std::fs::symlink_metadata(path).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let size_bytes = if is_dir {
        0
    } else {
        md.as_ref().map(|m| m.len()).unwrap_or(0)
    };
    let mtime = md
        .as_ref()
        .and_then(|m| m.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let size = if is_dir {
        "--".to_string()
    } else {
        human_size(size_bytes)
    };
    let kind = kind_of(path, is_dir);
    Some(Entry {
        name: name.into(),
        path: path.to_path_buf(),
        is_dir,
        size: size.into(),
        modified: date_label(mtime).into(),
        kind: kind.into(),
        size_bytes,
        mtime,
        search_detail: None,
    })
}

pub(super) fn read_entries(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            if let Some(entry) = entry_for(&e.path()) {
                v.push(entry);
            }
        }
    }
    v
}

pub(super) fn read_entries_checked(
    directory: &Path,
    show_hidden: bool,
    expected: Option<directory_state::Identity>,
) -> std::io::Result<(directory_state::Identity, Vec<Entry>)> {
    let before = directory_state::Identity::capture(directory)?;
    if expected.is_some_and(|expected| expected != before) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "the directory identity changed",
        ));
    }
    let mut entries = Vec::new();
    for result in std::fs::read_dir(directory)? {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if let Some(entry) = entry_for(&entry.path()) {
            entries.push(entry);
        }
    }
    if !before.still_matches(directory)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "the directory changed while it was read",
        ));
    }
    Ok((before, entries))
}

pub(super) fn disappeared_mount_roots(
    previous: &[rmac_mounts::Mount],
    current: &[rmac_mounts::Mount],
) -> Vec<PathBuf> {
    previous
        .iter()
        .filter(|old| {
            !current.iter().any(|new| {
                old.identity == new.identity
                    && old.path == new.path
                    && old.ejectable == new.ejectable
            })
        })
        .map(|mount| mount.path.clone())
        .collect()
}

pub(super) fn sort_entries(v: &mut [Entry], key: SortKey, asc: bool) {
    v.sort_by(|a, b| {
        let o = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Date => a.mtime.cmp(&b.mtime),
            SortKey::Size => a.size_bytes.cmp(&b.size_bytes),
            SortKey::Kind => a.kind.to_lowercase().cmp(&b.kind.to_lowercase()),
        };
        if asc {
            o
        } else {
            o.reverse()
        }
    });
}

pub(super) fn file_info(e: &Entry) -> Vec<(&'static str, String)> {
    let md = std::fs::metadata(&e.path).ok();
    let mut v: Vec<(&'static str, String)> = vec![("Kind", e.kind.to_string())];
    if !e.is_dir {
        v.push(("Size", format!("{} ({} bytes)", e.size, e.size_bytes)));
    }
    if let Some(parent) = e.path.parent() {
        v.push(("Where", parent.display().to_string()));
    }
    if let Some(md) = &md {
        if let Ok(created) = md.created() {
            v.push(("Created", date_label(created)));
        }
    }
    v.push(("Modified", e.modified.to_string()));
    if let Some(md) = &md {
        v.push(("Permissions", perm_string(md.permissions().mode())));
    }
    if let Ok(out) = Command::new("stat")
        .args(["-f", "%Su\n%Sg", &e.path.to_string_lossy()])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        let mut lines = s.lines();
        if let Some(o) = lines.next().filter(|l| !l.is_empty()) {
            v.push(("Owner", o.to_string()));
        }
        if let Some(g) = lines.next().filter(|l| !l.is_empty()) {
            v.push(("Group", g.to_string()));
        }
    }
    v
}

fn perm_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0b111;
        s.push(if bits & 0b100 != 0 { 'r' } else { '-' });
        s.push(if bits & 0b010 != 0 { 'w' } else { '-' });
        s.push(if bits & 0b001 != 0 { 'x' } else { '-' });
    }
    s
}

pub(super) fn free_space(path: &Path) -> Option<u64> {
    let stats = rustix::fs::statvfs(path).ok()?;
    let fragment_size = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    stats.f_bavail.checked_mul(fragment_size)
}

pub(super) fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K {
        format!("{:.2} GB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.1} MB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KB", b / K)
    } else {
        format!("{bytes} bytes")
    }
}

fn kind_of(path: &Path, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "Rust Source".into(),
        "toml" => "TOML Document".into(),
        "md" => "Markdown Document".into(),
        "txt" => "Plain Text Document".into(),
        "json" => "JSON document".into(),
        "lock" => "Document".into(),
        "png" => "PNG image".into(),
        "jpg" | "jpeg" => "JPEG image".into(),
        "gif" => "GIF image".into(),
        "webp" => "WebP image".into(),
        "pdf" => "PDF document".into(),
        "zip" => "ZIP archive".into(),
        "gz" | "tar" => "Archive".into(),
        "app" => "Application".into(),
        "" => "Document".into(),
        other => format!("{} document", other.to_uppercase()),
    }
}

fn date_label(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    let now = Local::now();
    let (h12, ap) = {
        let h = dt.hour();
        if h == 0 {
            (12, "AM")
        } else if h < 12 {
            (h, "AM")
        } else if h == 12 {
            (12, "PM")
        } else {
            (h - 12, "PM")
        }
    };
    let time = format!("{}:{:02} {}", h12, dt.minute(), ap);
    let days = now
        .date_naive()
        .signed_duration_since(dt.date_naive())
        .num_days();
    if days == 0 {
        format!("Today at {time}")
    } else if days == 1 {
        format!("Yesterday at {time}")
    } else {
        format!("{} {} {} at {time}", dt.day(), dt.format("%b"), dt.year())
    }
}
