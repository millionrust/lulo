use super::*;

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
        application: None,
    })
}

pub(super) fn entry_for_application(application: rmac_apps::Application) -> Entry {
    let metadata = std::fs::metadata(&application.source).ok();
    let mtime = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Entry {
        name: application.name.into(),
        path: application.source,
        is_dir: false,
        size: "--".into(),
        modified: date_label(mtime).into(),
        kind: "Application".into(),
        size_bytes: metadata.map_or(0, |metadata| metadata.len()),
        mtime,
        search_detail: application.generic_name.map(Into::into),
        application: Some(ApplicationEntry {
            launch: application.launch,
            icon: application.icon,
        }),
    }
}

pub(super) fn suppress_replaced_applications(
    catalog: Vec<rmac_apps::Application>,
) -> Vec<rmac_apps::Application> {
    let first_party_names = catalog
        .iter()
        .filter(|application| {
            let app_id = application
                .id
                .strip_suffix(".desktop")
                .unwrap_or(&application.id);
            rmac_apps::identity::ALL.contains(&app_id)
        })
        .map(|application| application.name.trim().to_lowercase())
        .collect::<BTreeSet<_>>();

    catalog
        .into_iter()
        .filter(|application| {
            let app_id = application
                .id
                .strip_suffix(".desktop")
                .unwrap_or(&application.id);
            rmac_apps::identity::ALL.contains(&app_id)
                || !first_party_names.contains(&application.name.trim().to_lowercase())
        })
        .collect()
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
    #[cfg(target_os = "macos")]
    let owner = Command::new("stat")
        .args(["-f", "%Su\n%Sg", &e.path.to_string_lossy()])
        .output();
    #[cfg(not(target_os = "macos"))]
    let owner = Command::new("stat")
        .args(["-c", "%U\n%G", &e.path.to_string_lossy()])
        .output();
    if let Ok(out) = owner {
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
    rmac_finder::listing::human_size(bytes)
}

fn kind_of(path: &Path, is_dir: bool) -> String {
    rmac_finder::listing::kind_of(path, is_dir)
}

fn date_label(t: SystemTime) -> String {
    rmac_finder::listing::date_label(t)
}
