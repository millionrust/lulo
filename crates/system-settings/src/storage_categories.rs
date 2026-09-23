//! Storage categories for the home volume, measured the way macOS Storage
//! does: by summing the sizes of the files that belong to each category.
//!
//! Only real measurements are reported. Each category walks its own folders
//! with `symlink_metadata`, never follows symbolic links, never crosses onto
//! another filesystem, and stops at a file-count bound (reported as
//! `truncated`) so a huge tree cannot stall the background executor.

use std::path::{Path, PathBuf};

/// Files visited per measurement before the walk stops and reports a lower
/// bound instead of an exact size.
pub(crate) const FILE_LIMIT: usize = 400_000;

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

/// Measure the home folder's categories on the filesystem that holds it.
/// Returns None when `$HOME` is unset or unreadable.
pub(crate) fn measure_home(volume_path: PathBuf) -> Option<Categories> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    measure(&home, volume_path)
}

/// Measure the categories under `home`, counting only files on `home`'s
/// own filesystem.
pub(crate) fn measure(home: &Path, volume_path: PathBuf) -> Option<Categories> {
    let device = device_of(home)?;
    let mut budget = FILE_LIMIT;
    let mut truncated = false;
    let mut categories = Vec::new();
    for (name, folders) in category_folders(home) {
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
            bytes = bytes.saturating_add(walk(&folder, device, &mut budget, &mut truncated));
        }
        if bytes > 0 {
            categories.push(Category {
                name,
                folder: first,
                bytes,
            });
        }
    }
    Some(Categories {
        volume_path,
        categories,
        truncated,
    })
}

fn walk(root: &Path, device: u64, budget: &mut usize, truncated: &mut bool) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                *truncated = true;
                return total;
            }
            *budget -= 1;
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() || device_of_metadata(&metadata) != device {
                continue;
            }
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(allocated(&metadata));
            }
        }
    }
    total
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
}
