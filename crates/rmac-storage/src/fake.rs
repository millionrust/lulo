//! In-memory [`Backend`] double for the settings/notes/theme/focus stores
//! built on `rmac-storage`.
//!
//! `InMemoryBackend` models a small POSIX-ish filesystem (files, and
//! directories that a write's parent must already exist in) without
//! touching disk. It does not model permissions, symlinks, or durability
//! (fsync) — those stay the host [`FileSystem`]'s job; see
//! [`crate::contract::assert_backend_contract`], which runs the same
//! assertions against both.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use super::*;

#[derive(Default)]
struct State {
    files: HashMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
}

/// An in-memory [`Backend`]. Every path lives under a root the caller
/// creates with [`InMemoryBackend::new`]; use [`with_dir`](Self::with_dir)
/// and [`with_file`](Self::with_file) to seed it.
pub struct InMemoryBackend {
    state: Mutex<State>,
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    /// Seeds a directory and every ancestor, matching `create_dir_all`.
    pub fn with_dir(self, path: impl Into<PathBuf>) -> Self {
        mark_dir_and_ancestors(&mut self.state.lock().unwrap().dirs, &path.into());
        self
    }

    /// Seeds a file and its parent directory chain.
    pub fn with_file(self, path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) -> Self {
        let path = path.into();
        let mut state = self.state.lock().unwrap();
        if let Some(parent) = path.parent() {
            mark_dir_and_ancestors(&mut state.dirs, parent);
        }
        state.files.insert(path, contents.into());
        self
    }
}

fn mark_dir_and_ancestors(dirs: &mut BTreeSet<PathBuf>, path: &Path) {
    let mut current = Some(path);
    while let Some(step) = current {
        if !dirs.insert(step.to_path_buf()) {
            break;
        }
        current = step.parent();
    }
}

fn is_root_like(path: &Path) -> bool {
    path.as_os_str().is_empty() || path == Path::new("/")
}

fn parent_dir_exists(dirs: &BTreeSet<PathBuf>, path: &Path) -> bool {
    match path.parent() {
        None => true,
        Some(parent) => is_root_like(parent) || dirs.contains(parent),
    }
}

fn not_found(what: &str, path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{what} not found: {}", path.display()),
    )
}

fn already_exists(what: &str, path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{what} already exists: {}", path.display()),
    )
}

fn no_parent_directory(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("parent directory does not exist: {}", path.display()),
    )
}

impl Backend for InMemoryBackend {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| not_found("file", path))
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if !parent_dir_exists(&state.dirs, path) {
            return Err(no_parent_directory(path));
        }
        state.files.insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    fn write_atomic_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        self.write_atomic(path, contents)
    }

    fn write_new_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.files.contains_key(path) {
            return Err(already_exists("file", path));
        }
        if !parent_dir_exists(&state.dirs, path) {
            return Err(no_parent_directory(path));
        }
        state.files.insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    fn write_new_private_stream(
        &self,
        path: &Path,
        source: &mut dyn io::Read,
        maximum: u64,
    ) -> io::Result<FileFingerprint> {
        let maximum = usize::try_from(maximum).unwrap_or(usize::MAX);
        let mut buffer = Vec::new();
        source.take(maximum as u64 + 1).read_to_end(&mut buffer)?;
        if buffer.len() > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stream exceeds the configured size limit",
            ));
        }
        let fingerprint = FileFingerprint {
            byte_len: buffer.len() as u64,
            sha256: Sha256::digest(&buffer).into(),
        };
        self.write_new_private(path, &buffer)?;
        Ok(fingerprint)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        mark_dir_and_ancestors(&mut self.state.lock().unwrap().dirs, path);
        Ok(())
    }

    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if !parent_dir_exists(&state.dirs, destination) {
            return Err(no_parent_directory(destination));
        }
        if let Some(contents) = state.files.remove(source) {
            state.files.insert(destination.to_path_buf(), contents);
            return Ok(());
        }
        if state.dirs.remove(source) {
            state.dirs.insert(destination.to_path_buf());
            let moved: Vec<PathBuf> = state
                .files
                .keys()
                .filter(|path| path.starts_with(source))
                .cloned()
                .collect();
            for old_path in moved {
                if let Ok(suffix) = old_path.strip_prefix(source) {
                    let new_path = destination.join(suffix);
                    if let Some(contents) = state.files.remove(&old_path) {
                        state.files.insert(new_path, contents);
                    }
                }
            }
            return Ok(());
        }
        Err(not_found("rename source", source))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.state
            .lock()
            .unwrap()
            .files
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| not_found("file", path))
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.dirs.contains(path) {
            return Err(not_found("directory", path));
        }
        state.dirs.retain(|dir| !dir.starts_with(path));
        state.files.retain(|file, _| !file.starts_with(path));
        Ok(())
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
        let mut state = self.state.lock().unwrap();
        let contents = state
            .files
            .get(source)
            .cloned()
            .ok_or_else(|| not_found("copy source", source))?;
        if state.files.contains_key(destination) {
            return Err(already_exists("copy destination", destination));
        }
        if !parent_dir_exists(&state.dirs, destination) {
            return Err(no_parent_directory(destination));
        }
        let len = contents.len() as u64;
        state.files.insert(destination.to_path_buf(), contents);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    #[test]
    fn fake_satisfies_the_shared_backend_contract() {
        let backend = InMemoryBackend::new().with_dir("/state");
        contract::assert_backend_contract(&backend, Path::new("/state"));
    }

    #[test]
    fn writing_without_a_parent_directory_fails() {
        let backend = InMemoryBackend::new();
        assert_eq!(
            backend
                .write_atomic(Path::new("/missing/file.json"), b"{}")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn write_new_private_never_replaces_an_existing_file() {
        let backend = InMemoryBackend::new()
            .with_dir("/state")
            .with_file("/state/a.bin", b"one".to_vec());
        assert_eq!(
            backend
                .write_new_private(Path::new("/state/a.bin"), b"two")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(backend.read(Path::new("/state/a.bin")).unwrap(), b"one");
    }

    #[test]
    fn remove_dir_all_drops_every_file_it_contains() {
        let backend = InMemoryBackend::new()
            .with_file("/state/notes/a.json", b"{}".to_vec())
            .with_file("/state/notes/b.json", b"{}".to_vec());
        backend.remove_dir_all(Path::new("/state/notes")).unwrap();
        assert!(backend.read(Path::new("/state/notes/a.json")).is_err());
    }
}
