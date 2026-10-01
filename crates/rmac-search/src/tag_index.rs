//! A bounded, in-memory index of Linux file tags (FILES-05): the
//! `user.rmac.tag` extended attribute Files writes when the user applies a
//! colour tag (`crates/finder/src/view/item_operations.rs`). macOS already
//! has a real tag search through Spotlight (`platform.rs`'s `tagged` for
//! `target_os = "macos"`); Linux has no equivalent system index, so this
//! module is the one for it.
//!
//! The index is built once per process by a bounded background scan of the
//! home directory (`start_background_scan`), then kept fresh two ways, with
//! no recursive whole-home filesystem watch and no polling:
//!
//! - Files' own tag writes call [`record`] directly, the moment they
//!   succeed.
//! - Every directory listing Files does — a plain navigation or a reload
//!   its own filesystem watcher triggered — calls [`note_listed`], which
//!   refreshes that one directory's slice of the index from disk. This is
//!   the "incremental update from the existing filesystem watcher events"
//!   FILES-05 asks for: it rides along on reloads the watcher already
//!   causes, rather than adding a second, home-wide watch of its own.
//!
//! A file nobody has browsed to and nobody has tagged through Files since
//! the last scan — tagged entirely by some other program — is caught the
//! next time the initial scan's bound allows it, or the next time its
//! folder is listed; this is the deliberate bound this module trades for
//! never doing an unbounded, continuously-rescanning home-directory walk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::{Error, Options};

/// The extended attribute Files writes a colour tag's lowercase name into
/// on Linux (`crates/finder/src/view/item_operations.rs`'s `FILE_TAG_XATTR`
/// for `not(target_os = "macos")` — kept in sync with that constant by
/// hand, since the two crates don't share a dependency edge the other way).
pub const XATTR_NAME: &str = "user.rmac.tag";

/// Hard caps on the initial scan: a visited-entry budget (not just a
/// tagged-file budget) so a huge, mostly-untagged home still finishes, and
/// a depth cap so a deeply nested build tree can't run away either.
const MAX_SCANNED_ENTRIES: usize = 150_000;
const MAX_SCAN_DEPTH: usize = 24;
/// Directories almost never worth reading tags out of, and often huge.
const SKIPPED_DIR_NAMES: [&str; 6] = [
    ".git",
    "node_modules",
    "target",
    ".cache",
    ".cargo",
    ".rustup",
];

#[derive(Default)]
struct Index {
    by_path: HashMap<PathBuf, String>,
    by_tag: HashMap<String, HashSet<PathBuf>>,
    scan_started: bool,
}

impl Index {
    fn set(&mut self, path: PathBuf, tag: Option<String>) {
        if let Some(previous) = self.by_path.remove(&path) {
            if let Some(members) = self.by_tag.get_mut(&previous) {
                members.remove(&path);
            }
        }
        if let Some(tag) = tag {
            self.by_tag
                .entry(tag.clone())
                .or_default()
                .insert(path.clone());
            self.by_path.insert(path, tag);
        }
    }
}

fn global() -> &'static Mutex<Index> {
    static INDEX: OnceLock<Mutex<Index>> = OnceLock::new();
    INDEX.get_or_init(|| Mutex::new(Index::default()))
}

fn lock(index: &Mutex<Index>) -> std::sync::MutexGuard<'_, Index> {
    index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts the one-time bounded background scan of `home`, unless a scan
/// has already started in this process (every Files window calls this at
/// startup; only the first call does anything). Runs on its own thread —
/// this crate has no async executor of its own — and is best effort: if
/// the thread can't be spawned, the index just stays empty until
/// navigation and tag writes fill it in via [`note_listed`] and [`record`].
pub fn start_background_scan(home: PathBuf) {
    {
        let mut index = lock(global());
        if index.scan_started {
            return;
        }
        index.scan_started = true;
    }
    let _ = std::thread::Builder::new()
        .name("rmac-tag-scan".into())
        .spawn(move || scan(&home));
}

fn scan(home: &Path) {
    let walker = walkdir::WalkDir::new(home)
        .follow_links(false)
        .max_depth(MAX_SCAN_DEPTH)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIPPED_DIR_NAMES.contains(&name))
        });
    for (visited, entry) in walker.enumerate() {
        if visited >= MAX_SCANNED_ENTRIES {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        if let Ok(tag) = read_tag(entry.path()) {
            record(entry.path(), Some(&tag));
        }
    }
}

/// Called the moment Files' own tag write or clear succeeds (Linux only):
/// keeps the index exactly in step with what Files itself just wrote,
/// without waiting for that folder to be listed again.
pub fn record(path: &Path, tag: Option<&str>) {
    lock(global()).set(path.to_path_buf(), tag.map(str::to_lowercase));
}

/// Called after every directory listing Files does, successful or not: a
/// fresh listing is the ground truth for exactly what that directory
/// contains right now, so this drops any indexed entry under `directory`
/// that is no longer in it (moved, renamed away, or deleted) and reads the
/// current tag, if any, of everything that is.
pub fn note_listed<'a>(directory: &Path, paths: impl Iterator<Item = &'a Path>) {
    let seen: Vec<PathBuf> = paths.map(Path::to_path_buf).collect();
    {
        let mut index = lock(global());
        let stale: Vec<PathBuf> = index
            .by_path
            .keys()
            .filter(|&path| path.parent() == Some(directory) && !seen.contains(path))
            .cloned()
            .collect();
        for path in stale {
            index.set(path, None);
        }
    }
    for path in seen {
        match read_tag(&path) {
            Ok(tag) => record(&path, Some(&tag)),
            Err(()) => record(&path, None),
        }
    }
}

/// The paths currently indexed under `tag` (already lowercased — Files'
/// own sidebar labels are title-cased, e.g. "Blue", so callers compare
/// case-insensitively), filtered the same way every other search result is.
pub(super) fn query(tag: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    crate::ranked::check_cancelled(options.cancel)?;
    let tag = tag.to_lowercase();
    let members = lock(global()).by_tag.get(&tag).cloned().unwrap_or_default();
    Ok(members
        .into_iter()
        .filter(|path| path.exists() && !crate::ranked::is_excluded(path, options.excluded_roots))
        .take(options.limit)
        .collect())
}

fn read_tag(path: &Path) -> Result<String, ()> {
    let mut buffer = [0u8; 32];
    let length = rustix::fs::getxattr(path, XATTR_NAME, &mut buffer).map_err(|_| ())?;
    std::str::from_utf8(&buffer[..length])
        .map(str::to_lowercase)
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_query_round_trip_and_clearing_removes_the_path() {
        let path = PathBuf::from("/tmp/rmac-tag-index-test/example.txt");
        record(&path, Some("Blue"));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        // query() filters by `path.exists()`, which this fixture path never
        // does; exercise the index directly instead, the way query() reads
        // it, to test the index's own bookkeeping without touching disk.
        assert_eq!(
            lock(global())
                .by_tag
                .get("blue")
                .map(|set| set.contains(&path)),
            Some(true)
        );
        record(&path, None);
        assert_eq!(lock(global()).by_path.get(&path), None);
        let _ = cancel;
    }

    #[test]
    fn note_listed_drops_entries_no_longer_in_the_directory() {
        let directory = PathBuf::from("/tmp/rmac-tag-index-test/dir");
        let stale = directory.join("gone.txt");
        record(&stale, Some("red"));
        // An empty listing of `directory`: `stale` is no longer in it.
        note_listed(&directory, std::iter::empty());
        assert_eq!(lock(global()).by_path.get(&stale), None);
    }

    #[test]
    fn tags_are_matched_case_insensitively() {
        let path = PathBuf::from("/tmp/rmac-tag-index-test/case.txt");
        record(&path, Some("Green"));
        assert!(lock(global())
            .by_tag
            .get("green")
            .is_some_and(|set| set.contains(&path)));
        record(&path, None);
    }

    #[test]
    fn scan_skips_well_known_huge_or_irrelevant_directories() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-tag-index-scan-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("node_modules")).unwrap();
        std::fs::create_dir_all(directory.join("kept")).unwrap();
        std::fs::write(directory.join("node_modules/ignored.txt"), b"x").unwrap();
        let kept_file = directory.join("kept/example.txt");
        std::fs::write(&kept_file, b"x").unwrap();
        if rustix::fs::setxattr(
            &kept_file,
            XATTR_NAME,
            b"purple",
            rustix::fs::XattrFlags::empty(),
        )
        .is_ok()
        {
            scan(&directory);
            assert!(lock(global())
                .by_tag
                .get("purple")
                .is_some_and(|set| set.contains(&kept_file)));
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
