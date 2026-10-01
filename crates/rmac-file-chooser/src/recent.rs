//! Recent chooser folders, scoped to the portal application's id.
//!
//! Disk access happens in the portal adapter, never on GPUI's render thread.
//! Paths are NUL-separated bytes so non-UTF-8 Unix names round trip.

use std::fs;
use std::io::Read as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 8;
const MAX_BYTES: u64 = 32 * 1024;

fn state_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })
        .map(|state| state.join("rmac/file-chooser"))
}

fn app_file(directory: &Path, app_id: &str) -> PathBuf {
    // Stable FNV-1a avoids using an untrusted app id as a filename.
    let hash = app_id.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    directory.join(format!("{hash:016x}"))
}

fn load_at(directory: &Path, app_id: &str) -> Vec<PathBuf> {
    let Ok(file) = fs::File::open(app_file(directory, app_id)) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES {
        return Vec::new();
    }
    bytes
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty() && bytes.len() <= 4096)
        .map(|bytes| PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
        .filter(|path| path.is_absolute() && path.is_dir())
        .take(MAX_RECENT)
        .collect()
}

pub fn load(app_id: &str) -> Vec<PathBuf> {
    state_directory()
        .map(|directory| load_at(&directory, app_id))
        .unwrap_or_default()
}

fn remember_at(directory: &Path, app_id: &str, folder: &Path) {
    if !folder.is_absolute() || !folder.is_dir() || folder.as_os_str().as_bytes().contains(&0) {
        return;
    }
    let mut paths = vec![folder.to_path_buf()];
    paths.extend(
        load_at(directory, app_id)
            .into_iter()
            .filter(|path| path != folder)
            .take(MAX_RECENT - 1),
    );
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path.as_os_str().as_bytes());
        bytes.push(0);
    }
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    let target = app_file(directory, app_id);
    let temporary = target.with_extension(format!("{}.tmp", std::process::id()));
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(&temporary, target);
    }
    let _ = fs::remove_file(temporary);
}

pub fn remember(app_id: &str, folder: &Path) {
    if let Some(directory) = state_directory() {
        remember_at(&directory, app_id, folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_folders_are_scoped_and_newest_first() {
        let root = std::env::temp_dir().join(format!("rmac-chooser-recent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        remember_at(&root, "app.one", &first);
        remember_at(&root, "app.one", &second);
        remember_at(&root, "app.one", &first);
        assert_eq!(load_at(&root, "app.one"), vec![first.clone(), second]);
        assert!(load_at(&root, "app.two").is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
