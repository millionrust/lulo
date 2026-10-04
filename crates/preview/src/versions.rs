//! File ▸ Revert To ▸ Last Opened for an edited image: a small, bounded
//! version store under XDG state, written once per session — before
//! Preview's first write to a document's own file — so Rotate/Flip/Adjust
//! Size/Crop followed by Save (or an autosave on close) is never
//! permanent. Mirrors `main.rs`'s `bookmarks_path` for the per-document
//! hash and its collision guard (a stored path mismatch means "not mine",
//! never "overwrite it"), and uses `rmac_storage`'s private-state
//! primitives for the actual writes/reads rather than hand-rolling them.
//!
//! One version is recorded per session that overwrites the file (gated by
//! `Slot::version_recorded`, not by this module), bounded by count, age
//! and total size on every write — see `prune_entries`. "Last Opened"
//! means the most recently recorded entry, i.e. what was on disk when the
//! *current* session's editing began; a session that never overwrites
//! the file records nothing, and reverting never discards or rewrites
//! older entries, mirroring the Mac's own Versions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::render::{self, Loaded};

/// Versions kept per document regardless of age.
const MAX_VERSIONS: usize = 5;
/// A version older than this is pruned on the next write, even if the
/// count is under `MAX_VERSIONS` — except the single most recent entry,
/// which is always kept so "Last Opened" never disappears out from under
/// a long-open session.
const MAX_AGE_SECS: u64 = 30 * 24 * 60 * 60;
/// Combined size of one document's kept versions.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// One version's own hard cap: an opened image is already bounded far
/// below this (`render::MAX_IMAGE_SIDE`), so this just backstops a
/// corrupt or unexpectedly huge source file.
const MAX_VERSION_BYTES: u64 = 256 * 1024 * 1024;
/// `meta.json` itself: a handful of small entries, never the image data.
const MAX_META_BYTES: usize = 64 * 1024;

#[derive(Default, Deserialize, Serialize)]
struct Store {
    /// The document this store belongs to — checked on every read.
    path: PathBuf,
    versions: Vec<Version>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Version {
    file: String,
    saved_at: u64,
    size: u64,
}

fn state_root() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("rmac-preview/versions")
}

/// FNV-1a of the absolute path, matching `main.rs`'s `bookmarks_path`.
fn hash_path(document: &Path) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    document
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
}

fn store_dir(root: &Path, document: &Path) -> PathBuf {
    root.join(format!("{:016x}", hash_path(document)))
}

fn meta_path(root: &Path, document: &Path) -> PathBuf {
    store_dir(root, document).join("meta.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn load(root: &Path, document: &Path) -> Option<Store> {
    let bytes = rmac_storage::read_bounded_no_follow(&meta_path(root, document), MAX_META_BYTES)
        .ok()?;
    let store: Store = serde_json::from_slice(&bytes).ok()?;
    (store.path == document).then_some(store)
}

/// Copies `document`'s current on-disk bytes into the version store. The
/// caller gates this on "has this slot already recorded a version this
/// session" (`Slot::version_recorded`) — this function itself always
/// records, unconditionally, and prunes afterwards.
pub fn record(document: &Path) -> Result<(), String> {
    record_under(&state_root(), document)
}

fn record_under(root: &Path, document: &Path) -> Result<(), String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let dir = store_dir(root, document);
    rmac_storage::create_dir_all_private(&dir).map_err(|error| error.to_string())?;
    let mut store = load(root, document).unwrap_or_else(|| Store {
        path: document.to_path_buf(),
        versions: Vec::new(),
    });
    // The user's own document, not Lulo's private state: no ownership or
    // single-hard-link requirement (`open_regular_no_follow`'s own doc
    // comment), unlike the `meta.json`/version-blob reads below.
    let mut source =
        rmac_storage::open_regular_no_follow(document).map_err(|error| error.to_string())?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!("{}-{}-{sequence}", now_secs(), std::process::id());
    let fingerprint =
        rmac_storage::write_new_private_stream(&dir.join(&name), &mut source, MAX_VERSION_BYTES)
            .map_err(|error| error.to_string())?;
    store.versions.push(Version {
        file: name,
        saved_at: now_secs(),
        size: fingerprint.byte_len,
    });
    for file in prune_entries(&mut store.versions, now_secs()) {
        let _ = std::fs::remove_file(dir.join(file));
    }
    let bytes = serde_json::to_vec(&store).map_err(|error| error.to_string())?;
    rmac_storage::atomic_write_private(&meta_path(root, document), &bytes)
        .map_err(|error| error.to_string())
}

/// Drops entries beyond `MAX_VERSIONS`, older than `MAX_AGE_SECS` (except
/// the single most recent) or past `MAX_TOTAL_BYTES` combined, oldest
/// first, and returns the dropped entries' file names for the caller to
/// delete. Pure (no filesystem access) so the bounding rules are
/// unit-testable on their own.
fn prune_entries(versions: &mut Vec<Version>, now: u64) -> Vec<String> {
    versions.sort_by_key(|version| version.saved_at);
    let mut removed = Vec::new();
    let last = versions.len().saturating_sub(1);
    let mut index = 0;
    versions.retain(|version| {
        let keep = index == last || now.saturating_sub(version.saved_at) <= MAX_AGE_SECS;
        index += 1;
        if !keep {
            removed.push(version.file.clone());
        }
        keep
    });
    while versions.len() > MAX_VERSIONS {
        removed.push(versions.remove(0).file);
    }
    let mut total: u64 = versions.iter().map(|version| version.size).sum();
    while total > MAX_TOTAL_BYTES && versions.len() > 1 {
        let dropped = versions.remove(0);
        total = total.saturating_sub(dropped.size);
        removed.push(dropped.file);
    }
    removed
}

/// The most recently recorded version for `document`, if any — what
/// Revert To ▸ Last Opened restores, and what enables that menu item
/// (cached on `Slot::has_version` rather than called from `render`: this
/// does disk I/O).
pub fn latest(document: &Path) -> Option<PathBuf> {
    latest_under(&state_root(), document)
}

fn latest_under(root: &Path, document: &Path) -> Option<PathBuf> {
    let store = load(root, document)?;
    let newest = store.versions.iter().max_by_key(|version| version.saved_at)?;
    Some(store_dir(root, document).join(&newest.file))
}

pub fn has_version(document: &Path) -> bool {
    latest(document).is_some()
}

/// File ▸ Revert To ▸ Last Opened: overwrites `document` with its most
/// recently recorded version, atomically, then reloads it so the caller
/// can refresh the in-memory `Slot`. Does not touch the version store —
/// the restored entry stays recorded, matching the Mac's own Revert,
/// which doesn't discard Versions history.
pub fn restore_latest(document: &Path) -> Result<Loaded, String> {
    restore_latest_under(&state_root(), document)
}

fn restore_latest_under(root: &Path, document: &Path) -> Result<Loaded, String> {
    let version = latest_under(root, document)
        .ok_or_else(|| "no earlier version is kept for this image".to_owned())?;
    let bytes = rmac_storage::read_bounded_no_follow(&version, MAX_VERSION_BYTES as usize)
        .map_err(|error| error.to_string())?;
    rmac_storage::atomic_write(document, &bytes).map_err(|error| error.to_string())?;
    render::load(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Content;

    fn pixels_of(loaded: &Loaded) -> &image::RgbaImage {
        match &loaded.content {
            Content::Image(image) => &image.pixels,
            Content::Pdf(_) => panic!("expected an image, got a PDF"),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-preview-versions-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn prune_entries_keeps_the_newest_within_count_age_and_size() {
        let now = 1_000_000;
        let mut versions = vec![
            Version { file: "a".into(), saved_at: now - 40 * 24 * 60 * 60, size: 10 },
            Version { file: "b".into(), saved_at: now - 10, size: 10 },
            Version { file: "c".into(), saved_at: now - 5, size: 10 },
        ];
        let removed = prune_entries(&mut versions, now);
        // "a" is older than MAX_AGE_SECS and is not the newest, so it's dropped.
        assert_eq!(removed, vec!["a".to_owned()]);
        assert_eq!(
            versions.iter().map(|v| v.file.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    #[test]
    fn prune_entries_never_drops_the_single_newest_even_if_stale_or_oversized() {
        // Genuinely stale (10x MAX_AGE_SECS) and far over MAX_TOTAL_BYTES.
        let now = MAX_AGE_SECS * 10;
        let mut versions = vec![Version {
            file: "only".into(),
            saved_at: 0,
            size: MAX_TOTAL_BYTES * 4,
        }];
        let removed = prune_entries(&mut versions, now);
        assert!(removed.is_empty());
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn prune_entries_caps_the_count_and_the_combined_size() {
        let now = 1_000_000;
        let mut versions: Vec<Version> = (0..8)
            .map(|index| Version {
                file: index.to_string(),
                saved_at: now - (8 - index) as u64,
                size: 1,
            })
            .collect();
        let removed = prune_entries(&mut versions, now);
        assert_eq!(removed, vec!["0", "1", "2"]);
        assert_eq!(versions.len(), MAX_VERSIONS);

        let mut big = vec![
            Version { file: "old".into(), saved_at: now - 2, size: MAX_TOTAL_BYTES },
            Version { file: "new".into(), saved_at: now - 1, size: MAX_TOTAL_BYTES },
        ];
        let removed = prune_entries(&mut big, now);
        assert_eq!(removed, vec!["old".to_owned()]);
        assert_eq!(big.len(), 1);
        assert_eq!(big[0].file, "new");
    }

    /// The round trip the coordinator asked for: record a version of a
    /// file, overwrite the file (simulating Save), then revert and check
    /// the original bytes come back — including through `restore_latest`'s
    /// own reload, which must see the image's real original pixels.
    #[test]
    fn overwrite_then_revert_round_trips_the_original_bytes() {
        let root = scratch("roundtrip");
        let _ = std::fs::remove_dir_all(&root);
        let document = root.join("source").join("photo.png");
        std::fs::create_dir_all(document.parent().unwrap()).unwrap();

        let original = swatch();
        render::save_image(&original, crate::document::ImageKind::Png, &document).unwrap();

        // Record "Last Opened" before the simulated Save overwrites it.
        record_under(&root, &document).unwrap();
        assert!(latest_under(&root, &document).is_some());

        // Simulate an edit + Save: a different image overwrites the file.
        let edited = render::flip_horizontal(&original);
        render::save_image(&edited, crate::document::ImageKind::Png, &document).unwrap();
        let after_save = render::load(&document).unwrap();
        assert_eq!(*pixels_of(&after_save), edited);

        // Revert restores exactly the pre-edit pixels.
        let reverted = restore_latest_under(&root, &document).unwrap();
        assert_eq!(*pixels_of(&reverted), original);
        let reread = render::load(&document).unwrap();
        assert_eq!(*pixels_of(&reread), original);

        // The restored version is still recorded (revert never discards
        // history), so a second revert is still possible.
        assert!(latest_under(&root, &document).is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_latest_without_any_recorded_version_fails_clearly() {
        let root = scratch("no-version");
        let _ = std::fs::remove_dir_all(&root);
        let document = root.join("source").join("untouched.png");
        std::fs::create_dir_all(document.parent().unwrap()).unwrap();
        render::save_image(&swatch(), crate::document::ImageKind::Png, &document).unwrap();

        assert!(latest_under(&root, &document).is_none());
        assert!(restore_latest_under(&root, &document).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A meta.json sitting at exactly the bucket `mine` would use, but
    /// recording a *different* path inside it — exactly what a real FNV
    /// hash collision with another document would look like from
    /// `mine`'s side — must read back as "no version", never as someone
    /// else's.
    #[test]
    fn a_stored_path_mismatch_is_treated_as_not_mine() {
        let root = scratch("collision");
        let _ = std::fs::remove_dir_all(&root);
        let mine = root.join("mine.png");
        let someone_elses_path = root.join("not-mine.png");
        let dir = store_dir(&root, &mine);
        std::fs::create_dir_all(&dir).unwrap();
        let bogus = Store {
            path: someone_elses_path.clone(),
            versions: vec![Version {
                file: "x".into(),
                saved_at: 1,
                size: 1,
            }],
        };
        let bytes = serde_json::to_vec(&bogus).unwrap();
        rmac_storage::atomic_write_private(&meta_path(&root, &mine), &bytes).unwrap();

        assert!(load(&root, &mine).is_none());
        assert!(latest_under(&root, &mine).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    fn swatch() -> image::RgbaImage {
        let mut pixels = image::RgbaImage::new(2, 2);
        pixels.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        pixels.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        pixels.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        pixels.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        pixels
    }
}
