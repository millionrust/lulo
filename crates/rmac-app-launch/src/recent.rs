//! Private, bounded, cross-process recent-application-launch authority.
//!
//! Every launch that goes through [`crate::launch`] and later reports success
//! records the launched app's stable catalog id here, in
//! `$XDG_STATE_HOME/rmac/recent-apps.json` (an atomic, versioned write with a
//! last-known-good backup, mirroring `rmac-recent-documents`). A surface such
//! as the Apps window reads this file once, when it opens, to build its
//! recents row; nothing polls it.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_ENTRIES: usize = 32;
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_ID_BYTES: usize = 512;
const LOCK_ATTEMPTS: usize = 200;
const LOCK_RETRY: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    Busy,
}

/// Recording or reading recent app launches failed. Always non-fatal to the
/// caller: a launch already happened (or is about to) whether or not this
/// succeeds, so callers log this rather than surface it as a launch failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecentLaunchError(ErrorKind);

impl std::fmt::Display for RecentLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("recent-application storage is unavailable")
    }
}

impl std::error::Error for RecentLaunchError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredEntry {
    id: String,
    used_at_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredFile {
    version: u32,
    entries: Vec<StoredEntry>,
}

impl Default for StoredFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

impl StoredFile {
    fn validate(&self) -> Result<(), RecentLaunchError> {
        if self.version != VERSION || self.entries.len() > MAX_ENTRIES {
            return Err(RecentLaunchError(ErrorKind::Invalid));
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.id.is_empty() || entry.id.len() > MAX_ID_BYTES)
        {
            return Err(RecentLaunchError(ErrorKind::Invalid));
        }
        Ok(())
    }
}

fn state_path() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })?;
    Some(state_home.join("rmac/recent-apps.json"))
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("last-good.json")
}

struct FileLock {
    _file: File,
}

impl FileLock {
    #[cfg(unix)]
    fn acquire(path: &Path) -> Result<Self, RecentLaunchError> {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
        use std::os::unix::io::AsRawFd as _;

        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let file = options
            .open(path)
            .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;
        let metadata = file
            .metadata()
            .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(RecentLaunchError(ErrorKind::Invalid));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;

        for _ in 0..LOCK_ATTEMPTS {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(RecentLaunchError(ErrorKind::Io(error.kind())));
            }
            std::thread::sleep(LOCK_RETRY);
        }
        Err(RecentLaunchError(ErrorKind::Busy))
    }

    #[cfg(not(unix))]
    fn acquire(_path: &Path) -> Result<Self, RecentLaunchError> {
        Err(RecentLaunchError(ErrorKind::Invalid))
    }
}

fn read(path: &Path) -> Result<StoredFile, RecentLaunchError> {
    let bytes = rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES)
        .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;
    let stored: StoredFile =
        serde_json::from_slice(&bytes).map_err(|_| RecentLaunchError(ErrorKind::Invalid))?;
    stored.validate()?;
    Ok(stored)
}

fn load_stored(path: &Path) -> StoredFile {
    match read(path) {
        Ok(stored) => stored,
        Err(_) => read(&backup_path(path)).unwrap_or_default(),
    }
}

fn save(path: &Path, stored: &StoredFile) -> Result<(), RecentLaunchError> {
    stored.validate()?;
    let bytes = serde_json::to_vec(stored).map_err(|_| RecentLaunchError(ErrorKind::Invalid))?;
    rmac_storage::atomic_write_private(&backup_path(path), &bytes)
        .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;
    rmac_storage::atomic_write_private(path, &bytes)
        .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// Record that `app_id` (a stable catalog id, e.g. a desktop-entry id with
/// its `.desktop` suffix trimmed) was just launched. Best-effort: a failure
/// here never means the launch itself failed.
pub fn record_recent_launch(app_id: &str) -> Result<(), RecentLaunchError> {
    let path = state_path().ok_or(RecentLaunchError(ErrorKind::Invalid))?;
    record_recent_launch_at(&path, app_id)
}

/// The most recently launched app ids, most recent first, deduplicated and
/// capped at `limit`. Never blocks on another process's write for long: a
/// busy or missing store simply reads as empty.
pub fn recent_app_ids(limit: usize) -> Vec<String> {
    let Some(path) = state_path() else {
        return Vec::new();
    };
    recent_app_ids_at(&path, limit)
}

/// The path-parameterized core of [`record_recent_launch`], kept separate
/// so tests can exercise it against a private temporary file instead of
/// racing other tests over the real, process-global `$XDG_STATE_HOME`.
fn record_recent_launch_at(path: &Path, app_id: &str) -> Result<(), RecentLaunchError> {
    if app_id.is_empty() || app_id.len() > MAX_ID_BYTES {
        return Err(RecentLaunchError(ErrorKind::Invalid));
    }
    let parent = path.parent().ok_or(RecentLaunchError(ErrorKind::Invalid))?;
    rmac_storage::create_dir_all_private(parent)
        .map_err(|error| RecentLaunchError(ErrorKind::Io(error.kind())))?;
    let _lock = FileLock::acquire(&parent.join("recent-apps.lock"))?;
    let mut stored = load_stored(path);
    stored.entries.retain(|entry| entry.id != app_id);
    stored.entries.insert(
        0,
        StoredEntry {
            id: app_id.to_owned(),
            used_at_unix_ms: current_unix_ms(),
        },
    );
    stored.entries.truncate(MAX_ENTRIES);
    save(path, &stored)
}

/// The path-parameterized core of [`recent_app_ids`]; see
/// [`record_recent_launch_at`] for why it takes a path.
fn recent_app_ids_at(path: &Path, limit: usize) -> Vec<String> {
    let mut stored = load_stored(path);
    stored
        .entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.used_at_unix_ms));
    stored
        .entries
        .into_iter()
        .map(|entry| entry.id)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_store_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-app-launch-recent-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn records_move_to_the_front_deduplicate_and_are_bounded() {
        let path = temp_store_path("basic").join("recent-apps.json");
        record_recent_launch_at(&path, "org.rmac.Files").unwrap();
        record_recent_launch_at(&path, "org.rmac.Notes").unwrap();
        record_recent_launch_at(&path, "org.rmac.Files").unwrap();
        assert_eq!(
            recent_app_ids_at(&path, 10),
            ["org.rmac.Files", "org.rmac.Notes"]
        );
        assert_eq!(recent_app_ids_at(&path, 1), ["org.rmac.Files"]);

        for index in 0..MAX_ENTRIES + 5 {
            record_recent_launch_at(&path, &format!("app-{index}")).unwrap();
        }
        assert_eq!(recent_app_ids_at(&path, MAX_ENTRIES + 5).len(), MAX_ENTRIES);
    }

    #[test]
    fn rejects_an_empty_or_oversized_id() {
        let path = temp_store_path("invalid").join("recent-apps.json");
        assert!(record_recent_launch_at(&path, "").is_err());
        assert!(record_recent_launch_at(&path, &"x".repeat(MAX_ID_BYTES + 1)).is_err());
        assert!(recent_app_ids_at(&path, 10).is_empty());
    }

    #[test]
    fn a_missing_store_reads_as_empty() {
        let path = temp_store_path("missing").join("recent-apps.json");
        assert!(recent_app_ids_at(&path, 10).is_empty());
    }
}
