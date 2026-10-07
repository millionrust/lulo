use std::io;
#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    device: u64,
    inode: u64,
}

#[cfg(not(windows))]
impl Identity {
    pub fn capture(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "the location is no longer a directory",
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub fn still_matches(self, path: &Path) -> io::Result<bool> {
        Ok(Self::capture(path)? == self)
    }
}

/// Windows has no inode, and the real per-directory identity that would
/// stand in for one (`MetadataExt::file_index`) is still the unstable
/// `windows_by_handle` feature (rust-lang/rust#63010) on this pinned
/// stable toolchain. The canonical (symlink/junction-resolved) path is the
/// real, stable alternative: two paths naming the same directory resolve
/// to the same canonical path, which is exactly what "did this survive a
/// rename" needs (ADR 0023 phase 4).
#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity(PathBuf);

#[cfg(windows)]
impl Identity {
    pub fn capture(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "the location is no longer a directory",
            ));
        }
        Ok(Self(path.canonicalize()?))
    }

    pub fn still_matches(&self, path: &Path) -> io::Result<bool> {
        Ok(&Self::capture(path)? == self)
    }
}

/// Resolve a rename hint only when the current directory's exact filesystem
/// identity appears below the hinted new path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameResolution {
    pub old: PathBuf,
    pub new: PathBuf,
    pub current: PathBuf,
}

pub fn renamed_path(
    current: &Path,
    expected: Identity,
    renames: &[(PathBuf, PathBuf)],
) -> Option<RenameResolution> {
    renames.iter().find_map(|(old, new)| {
        let relative = current.strip_prefix(old).ok()?;
        let candidate = new.join(relative);
        expected
            .still_matches(&candidate)
            .ok()
            .filter(|matches| *matches)
            .map(|_| RenameResolution {
                old: old.clone(),
                new: new.clone(),
                current: candidate,
            })
    })
}

/// Find the closest accessible parent after the current directory disappears
/// or is replaced. `home` is preferred once no parent below it survives; `/`
/// is the final truthful fallback.
pub fn recovery_parent(current: &Path, home: &Path) -> PathBuf {
    let mut candidate = current.parent();
    while let Some(path) = candidate {
        if accessible_directory(path) {
            return path.to_path_buf();
        }
        candidate = path.parent();
    }
    if accessible_directory(home) {
        home.to_path_buf()
    } else {
        PathBuf::from("/")
    }
}

pub fn rewrite_prefix(path: &Path, old: &Path, new: &Path) -> PathBuf {
    path.strip_prefix(old)
        .map(|relative| new.join(relative))
        .unwrap_or_else(|_| path.to_path_buf())
}

pub fn lies_under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn accessible_directory(path: &Path) -> bool {
    Identity::capture(path).is_ok() && std::fs::read_dir(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Windows' canonical-path `Identity` cannot witness a rename survive:
    // it resolves to the path as it stands *now*, so capturing it before
    // the rename and re-capturing the moved folder afterwards necessarily
    // gives two different strings, unlike Unix's inode, which is the same
    // object either way. "Files' window follows a folder renamed out from
    // under it" is therefore an accepted gap on Windows (WIN-OS-20) rather
    // than something this pinned-stable-toolchain identity can fix.
    #[test]
    #[cfg(unix)]
    #[allow(clippy::clone_on_copy)] // `Identity` is `Copy` here but not on Windows.
    fn rename_resolution_requires_the_exact_directory_identity() {
        let root = temporary_directory("rename");
        let old = root.join("old");
        let child = old.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let expected = Identity::capture(&child).unwrap();
        let new = root.join("new");
        std::fs::rename(&old, &new).unwrap();

        assert_eq!(
            renamed_path(&child, expected.clone(), &[(old.clone(), new.clone())]),
            Some(RenameResolution {
                old: old.clone(),
                new: new.clone(),
                current: new.join("child"),
            })
        );
        std::fs::create_dir_all(&old).unwrap();
        assert!(!expected.still_matches(&old).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_uses_the_nearest_accessible_parent() {
        let root = temporary_directory("parent");
        let vanished = root.join("one").join("two");
        std::fs::create_dir_all(&vanished).unwrap();
        std::fs::remove_dir(&vanished).unwrap();

        assert_eq!(recovery_parent(&vanished, &root), root.join("one"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prefix_rewrite_and_root_membership_are_component_aware() {
        assert_eq!(
            rewrite_prefix(
                Path::new("/media/disk/docs"),
                Path::new("/media/disk"),
                Path::new("/run/media/disk")
            ),
            PathBuf::from("/run/media/disk/docs")
        );
        assert!(lies_under_any(
            Path::new("/media/disk/docs"),
            &[PathBuf::from("/media/disk")]
        ));
        assert!(!lies_under_any(
            Path::new("/media/diskette"),
            &[PathBuf::from("/media/disk")]
        ));
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-directory-state-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
