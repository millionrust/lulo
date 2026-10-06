//! The Dock's early application entries, cached between logins (SPEED-05).
//!
//! At login the Dock can show nothing until it has catalog entries for the
//! apps it keeps (pinned) and remembers (recent). Parsing them takes a scan
//! of every application directory plus icon-theme lookups; the whole catalog
//! takes ~250 ms on the reference laptop. The entries the Dock showed last
//! time are kept in `$XDG_CACHE_HOME/rmac/dock-entries.json` with each desktop
//! entry's modification time. The next login uses them when every wanted ID
//! is cached, every desktop entry still has its recorded modification time
//! and every icon still exists (one `stat` each), so the first snapshot has
//! the same entries -- and the same Dock size -- as the full catalog that
//! follows and reconciles anything else.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct CachedEntry {
    application: rmac_apps::Application,
    /// Nanoseconds since the epoch of the desktop entry's mtime.
    modified: u128,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct EntryCache {
    entries: Vec<CachedEntry>,
}

fn cache_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".cache"))
        })
        .map(|cache| cache.join("rmac/dock-entries.json"))
}

fn modified(path: &Path) -> Option<u128> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(time.duration_since(UNIX_EPOCH).ok()?.as_nanos())
}

fn wanted(application: &rmac_apps::Application, ids: &[String]) -> bool {
    ids.iter().any(|id| {
        *id == application.id || application.id.strip_suffix(".desktop") == Some(id.as_str())
    })
}

/// The cached entries for `ids`, when the cache covers every one of them and
/// nothing it recorded has changed on disk; otherwise `None`.
fn load_from(path: &Path, ids: &[String]) -> Option<Vec<rmac_apps::Application>> {
    let cache: EntryCache = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let mut entries = Vec::new();
    for id in ids {
        let entry = cache
            .entries
            .iter()
            .find(|entry| wanted(&entry.application, std::slice::from_ref(id)))?;
        if modified(&entry.application.source) != Some(entry.modified) {
            return None;
        }
        if let Some(icon) = &entry.application.icon {
            if !icon.exists() {
                return None;
            }
        }
        if !entries.contains(&entry.application) {
            entries.push(entry.application.clone());
        }
    }
    Some(entries)
}

/// Record the entries for `ids` out of `applications`, replacing the cache
/// only when its contents change.
fn save_to(path: &Path, ids: &[String], applications: &[rmac_apps::Application]) {
    let cache = EntryCache {
        entries: applications
            .iter()
            .filter(|application| wanted(application, ids))
            .filter_map(|application| {
                Some(CachedEntry {
                    modified: modified(&application.source)?,
                    application: application.clone(),
                })
            })
            .collect(),
    };
    let Ok(bytes) = serde_json::to_vec(&cache) else {
        return;
    };
    if std::fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rmac_storage::atomic_write(path, &bytes);
}

/// The IDs the Dock shows before the full catalog: pinned, then recent.
pub(super) fn early_ids() -> Vec<String> {
    let mut ids: Vec<String> = rmac_shell_settings::ShellSettingsStore::from_environment()
        .ok()
        .and_then(|store| store.load().ok())
        .map(|snapshot| {
            snapshot
                .settings
                .pinned_apps
                .into_iter()
                .map(|app| app.0)
                .collect()
        })
        .unwrap_or_default();
    for recent in super::load_recents() {
        if !ids.contains(&recent) {
            ids.push(recent);
        }
    }
    ids
}

/// The entries for `ids`: from the cache when it is still valid, otherwise
/// parsed from just those desktop entries (and cached). `None` when there is
/// nothing to show early.
pub(super) fn early_entries(ids: &[String]) -> Option<Vec<rmac_apps::Application>> {
    if ids.is_empty() {
        return None;
    }
    let path = cache_path();
    if let Some(entries) = path.as_deref().and_then(|path| load_from(path, ids)) {
        return Some(entries);
    }
    let entries = rmac_apps::discover_entries(ids).ok()?;
    if let Some(path) = &path {
        save_to(path, ids, &entries);
    }
    (!entries.is_empty()).then_some(entries)
}

/// Refresh the cache from a full catalog read.
pub(super) fn refresh(ids: &[String], catalog: &[rmac_apps::Application]) {
    if let Some(path) = cache_path() {
        save_to(&path, ids, catalog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(id: &str, source: &Path, icon: Option<PathBuf>) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: id.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: source.to_path_buf(),
            icon,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: "/bin/true".into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn cached_entries_are_used_until_a_desktop_entry_or_icon_changes() {
        let root = std::env::temp_dir().join(format!(
            "rmac-dock-entry-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let desktop = root.join("files.desktop");
        std::fs::write(&desktop, "[Desktop Entry]\n").unwrap();
        let icon = root.join("files.svg");
        std::fs::write(&icon, "<svg/>").unwrap();
        let cache = root.join("cache.json");
        let files = application("files.desktop", &desktop, Some(icon.clone()));
        let ids = vec!["files".to_owned()];

        assert!(load_from(&cache, &ids).is_none());
        save_to(&cache, &ids, std::slice::from_ref(&files));
        assert_eq!(load_from(&cache, &ids), Some(vec![files.clone()]));

        // An ID the cache never saw: fall back to parsing.
        assert!(load_from(&cache, &["mail".to_owned()]).is_none());

        // The icon disappeared.
        std::fs::remove_file(&icon).unwrap();
        assert!(load_from(&cache, &ids).is_none());
        std::fs::write(&icon, "<svg/>").unwrap();
        assert!(load_from(&cache, &ids).is_some());

        // The desktop entry was edited.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&desktop, "[Desktop Entry]\nName=Files\n").unwrap();
        assert!(load_from(&cache, &ids).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
