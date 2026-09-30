//! Storage categories for the home volume, measured the way macOS Storage
//! does: by summing the sizes of the files that belong to each category.
//!
//! Only real measurements are reported. The scan runs on the blocking pool,
//! never follows symbolic links or crosses filesystems, and can be cancelled.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

const CACHE_VERSION: u32 = 1;
const CACHE_MAX_BYTES: usize = 16 * 1024;
pub(crate) const REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);

/// Files visited per measurement before the walk stops and reports a lower
/// bound instead of an exact size. Keeping this bounded makes opening Storage
/// responsive on home folders with large application caches and Flatpak data.
pub(crate) const FILE_LIMIT: usize = 64_000;

/// One measured category of the home volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Category {
    pub(crate) name: &'static str,
    /// The folder a reveal opens (the first existing folder of the category).
    pub(crate) folder: Option<PathBuf>,
    pub(crate) bytes: u64,
}

/// The measured categories of one volume plus what they leave unexplained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Categories {
    /// The mount point these categories were measured on.
    pub(crate) volume_path: PathBuf,
    /// Categories with at least one byte, in the Mac's order.
    pub(crate) categories: Vec<Category>,
    /// True when the file bound was reached; sizes are then lower bounds.
    pub(crate) truncated: bool,
}

impl Categories {
    /// Everything used on the volume that no category accounts for, which
    /// macOS labels "System Data".
    pub(crate) fn system_data(&self, used: u64) -> u64 {
        used.saturating_sub(self.categories.iter().map(|category| category.bytes).sum())
    }
}

/// The category layout under a home folder, in the Mac's order.
pub(crate) fn category_folders(home: &Path) -> Vec<(&'static str, Vec<PathBuf>)> {
    vec![
        (
            "Applications",
            vec![home.join(".local/share/flatpak"), home.join(".var/app")],
        ),
        (
            "Documents",
            vec![
                home.join("Documents"),
                home.join("Desktop"),
                home.join("Downloads"),
            ],
        ),
        ("Music", vec![home.join("Music")]),
        ("Photos", vec![home.join("Pictures")]),
        ("Movies", vec![home.join("Videos")]),
        ("Bin", vec![home.join(".local/share/Trash")]),
    ]
}

/// Measure the categories under `home`, counting only files on `home`'s
/// own filesystem.
#[cfg(test)]
pub(crate) fn measure(home: &Path, volume_path: PathBuf) -> Option<Categories> {
    measure_with_progress(home, volume_path, &AtomicBool::new(false), |_| {})
}

pub(crate) fn measure_with_progress(
    home: &Path,
    volume_path: PathBuf,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(Categories),
) -> Option<Categories> {
    let device = device_of(home)?;
    let mut budget = FILE_LIMIT;
    let mut truncated = false;
    let mut categories = Vec::new();
    for (name, folders) in category_folders(home) {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let mut bytes = 0u64;
        let mut first = None;
        for folder in folders {
            let Ok(metadata) = std::fs::symlink_metadata(&folder) else {
                continue;
            };
            if !metadata.is_dir() || device_of_metadata(&metadata) != device {
                continue;
            }
            first.get_or_insert_with(|| folder.clone());
            bytes = bytes.saturating_add(walk(
                &folder,
                device,
                &mut budget,
                &mut truncated,
                cancelled,
            )?);
        }
        if bytes > 0 {
            categories.push(Category {
                name,
                folder: first,
                bytes,
            });
        }
        progress(Categories {
            volume_path: volume_path.clone(),
            categories: categories.clone(),
            truncated,
        });
    }
    Some(Categories {
        volume_path,
        categories,
        truncated,
    })
}

fn walk(
    root: &Path,
    device: u64,
    budget: &mut usize,
    truncated: &mut bool,
    cancelled: &AtomicBool,
) -> Option<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if cancelled.load(Ordering::Relaxed) {
                return None;
            }
            if *budget == 0 {
                *truncated = true;
                return Some(total);
            }
            *budget -= 1;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if is_bulk_directory(&entry.file_name()) {
                    // These bytes already belong to the volume's used total;
                    // leaving them in residual System Data avoids another walk.
                    *truncated = true;
                    continue;
                }
            } else if !file_type.is_file() {
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if device_of_metadata(&metadata) != device {
                continue;
            }
            if file_type.is_dir() && metadata.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() && metadata.is_file() {
                total = total.saturating_add(allocated(&metadata));
            }
        }
    }
    Some(total)
}

fn is_bulk_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "node_modules" | "target" | ".cache" | "Cache" | "Caches")
    )
}

#[derive(Serialize, Deserialize)]
struct StoredCategories {
    version: u32,
    measured_at: u64,
    home: PathBuf,
    volume_path: PathBuf,
    categories: Vec<StoredCategory>,
    truncated: bool,
}

#[derive(Serialize, Deserialize)]
struct StoredCategory {
    name: String,
    folder: Option<PathBuf>,
    bytes: u64,
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("rmac/system-settings/storage-categories.json"))
}

/// Read a bounded, versioned snapshot. A matching snapshot can be shown before
/// the next scan; its age decides whether that scan is needed.
pub(crate) fn load_cache(home: &Path, volume_path: &Path) -> Option<(Categories, bool)> {
    load_cache_at(&cache_path()?, home, volume_path)
}

fn load_cache_at(path: &Path, home: &Path, volume_path: &Path) -> Option<(Categories, bool)> {
    let bytes = FileSystem
        .read_bounded_no_follow(path, CACHE_MAX_BYTES)
        .ok()?;
    let stored: StoredCategories = serde_json::from_slice(&bytes).ok()?;
    if stored.version != CACHE_VERSION
        || stored.home != home
        || stored.volume_path != volume_path
        || stored.categories.len() > 6
    {
        return None;
    }
    let layout = category_folders(home);
    let mut categories = Vec::new();
    for item in stored.categories {
        let name = layout.iter().find(|(name, _)| *name == item.name)?.0;
        if item
            .folder
            .as_ref()
            .is_some_and(|folder| !folder.starts_with(home))
        {
            return None;
        }
        categories.push(Category {
            name,
            folder: item.folder,
            bytes: item.bytes,
        });
    }
    let fresh = stored.measured_at <= now_secs()
        && now_secs().saturating_sub(stored.measured_at) < REFRESH_AFTER.as_secs();
    Some((
        Categories {
            volume_path: stored.volume_path,
            categories,
            truncated: stored.truncated,
        },
        fresh,
    ))
}

pub(crate) fn save_cache(home: &Path, categories: &Categories) -> std::io::Result<()> {
    let path = cache_path().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    save_cache_at(&path, home, categories)
}

fn save_cache_at(path: &Path, home: &Path, categories: &Categories) -> std::io::Result<()> {
    let stored = StoredCategories {
        version: CACHE_VERSION,
        measured_at: now_secs(),
        home: home.to_path_buf(),
        volume_path: categories.volume_path.clone(),
        categories: categories
            .categories
            .iter()
            .map(|category| StoredCategory {
                name: category.name.to_owned(),
                folder: category.folder.clone(),
                bytes: category.bytes,
            })
            .collect(),
        truncated: categories.truncated,
    };
    let bytes = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    if bytes.len() > CACHE_MAX_BYTES {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    rmac_storage::create_dir_all_private(path.parent().unwrap())?;
    rmac_storage::atomic_write_private(&path, &bytes)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Lower this blocking worker's CPU and disk scheduling priority on Linux.
pub(crate) fn lower_scan_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        libc::syscall(libc::SYS_ioprio_set, 1, 0, (7 << 13) | 7);
    }
}

#[cfg(unix)]
fn device_of_metadata(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.dev()
}

#[cfg(not(unix))]
fn device_of_metadata(_metadata: &std::fs::Metadata) -> u64 {
    0
}

/// Bytes a file occupies on disk (sparse files count what they use).
#[cfg(unix)]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.len().min(metadata.blocks().saturating_mul(512))
}

#[cfg(not(unix))]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

fn device_of(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|metadata| device_of_metadata(&metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rmac-storage-categories-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sums_each_category_and_skips_empty_ones() {
        let home = scratch("sums");
        std::fs::create_dir_all(home.join("Documents/nested")).unwrap();
        std::fs::create_dir_all(home.join("Music")).unwrap();
        std::fs::write(home.join("Documents/a.txt"), vec![7u8; 10_000]).unwrap();
        std::fs::write(home.join("Documents/nested/b.txt"), vec![7u8; 10_000]).unwrap();
        let measured = measure(&home, home.clone()).unwrap();
        let names: Vec<_> = measured.categories.iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["Documents"]);
        assert!(measured.categories[0].bytes > 0);
        assert!(measured.categories[0].bytes <= 20_000);
        assert_eq!(measured.categories[0].folder, Some(home.join("Documents")));
        assert!(!measured.truncated);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_symbolic_links() {
        let home = scratch("links");
        let outside = scratch("links-outside");
        std::fs::write(outside.join("big.bin"), vec![1u8; 50_000]).unwrap();
        std::fs::create_dir_all(home.join("Pictures")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("Pictures/elsewhere")).unwrap();
        let measured = measure(&home, home.clone()).unwrap();
        assert!(measured.categories.is_empty());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn system_data_is_what_categories_leave() {
        let categories = Categories {
            volume_path: PathBuf::from("/"),
            categories: vec![Category {
                name: "Music",
                folder: None,
                bytes: 30,
            }],
            truncated: false,
        };
        assert_eq!(categories.system_data(100), 70);
        assert_eq!(categories.system_data(10), 0);
    }

    #[test]
    fn skips_bulk_directories_and_streams_completed_categories() {
        let home = scratch("bulk");
        std::fs::create_dir_all(home.join("Documents/node_modules/pkg")).unwrap();
        std::fs::write(home.join("Documents/letter.txt"), vec![1u8; 8192]).unwrap();
        std::fs::write(
            home.join("Documents/node_modules/pkg/bundle.js"),
            vec![1u8; 8192],
        )
        .unwrap();
        let mut updates = Vec::new();
        let measured =
            measure_with_progress(&home, home.clone(), &AtomicBool::new(false), |partial| {
                updates.push(partial);
            })
            .unwrap();
        assert_eq!(updates.len(), category_folders(&home).len());
        assert_eq!(updates.last(), Some(&measured));
        assert!(measured.truncated);
        assert_eq!(measured.categories.len(), 1);
        assert_eq!(measured.categories[0].name, "Documents");
        assert!(measured.categories[0].bytes <= 8192);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cancellation_stops_before_scanning() {
        let home = scratch("cancel");
        let cancelled = AtomicBool::new(true);
        assert!(
            measure_with_progress(&home, home.clone(), &cancelled, |_| panic!(
                "progress after cancellation"
            ))
            .is_none()
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cancellation_after_progress_skips_remaining_categories() {
        let home = scratch("cancel-progress");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        let cancelled = AtomicBool::new(false);
        let mut updates = 0;
        let result = measure_with_progress(&home, home.clone(), &cancelled, |_| {
            updates += 1;
            cancelled.store(true, Ordering::Relaxed);
        });
        assert!(result.is_none());
        assert_eq!(updates, 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cache_round_trip_rejects_other_home_and_version() {
        let home = scratch("cache");
        let path = home.join("state/categories.json");
        let measured = Categories {
            volume_path: PathBuf::from("/"),
            categories: vec![Category {
                name: "Documents",
                folder: Some(home.join("Documents")),
                bytes: 4096,
            }],
            truncated: false,
        };
        save_cache_at(&path, &home, &measured).unwrap();
        assert_eq!(
            load_cache_at(&path, &home, Path::new("/")),
            Some((measured.clone(), true))
        );
        assert!(load_cache_at(&path, Path::new("/other"), Path::new("/")).is_none());
        let mut stored: StoredCategories =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        stored.measured_at = now_secs() - REFRESH_AFTER.as_secs() - 1;
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(
            load_cache_at(&path, &home, Path::new("/")).map(|(_, fresh)| fresh),
            Some(false)
        );
        stored.version += 1;
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load_cache_at(&path, &home, Path::new("/")).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}
