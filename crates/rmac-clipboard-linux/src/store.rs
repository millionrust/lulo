//! Private on-disk state: payloads and the index in a 0700 directory under
//! `$XDG_RUNTIME_DIR` (a per-user tmpfs that logind removes at logout), and
//! the user's Allow choice in `$XDG_CONFIG_HOME/rmac/clipboard.json`.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use rmac_clipboard::History;
use serde::{Deserialize, Serialize};

use crate::Error;

const INDEX: &str = "index.json";
const PAYLOAD_PREFIX: &str = "item-";

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
    config: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Config {
    enabled: bool,
}

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(Error::Store)?;
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|home| home.join(".config"))
            })
            .ok_or(Error::Store)?;
        Ok(Self::at(
            runtime.join("rmac/clipboard"),
            config_home.join("rmac/clipboard.json"),
        ))
    }

    pub fn at(dir: PathBuf, config: PathBuf) -> Self {
        Self { dir, config }
    }

    /// Create the private directory, or tighten it to 0700 if it exists.
    /// A symlink in its place is refused.
    pub fn prepare(&self) -> Result<(), Error> {
        if let Some(parent) = self.dir.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|_| Error::Store)?;
        }
        match fs::symlink_metadata(&self.dir) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(Error::Store),
            Err(_) => DirBuilder::new()
                .mode(0o700)
                .create(&self.dir)
                .map_err(|_| Error::Store)?,
        }
        fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Store)
    }

    /// The saved history with its bounds re-applied. Payload files that no
    /// entry refers to (a crash between writes) are deleted.
    pub fn load_history(&self) -> History {
        let mut history = fs::read(self.dir.join(INDEX))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<History>(&bytes).ok())
            .unwrap_or_default();
        let dropped = history.normalise();
        self.remove_payloads(&dropped);
        let known = history
            .entries()
            .iter()
            .map(|entry| entry.id)
            .collect::<std::collections::BTreeSet<_>>();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(id) = name
                    .to_str()
                    .and_then(|name| name.strip_prefix(PAYLOAD_PREFIX))
                    .and_then(|id| id.parse::<u64>().ok())
                else {
                    continue;
                };
                if !known.contains(&id) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        history.retain_payloads(|id| self.payload_path(id).is_file());
        history
    }

    pub fn save_history(&self, history: &History) -> Result<(), Error> {
        let bytes = serde_json::to_vec(history).map_err(|_| Error::Store)?;
        write_private(&self.dir.join(INDEX), &bytes)
    }

    pub fn payload_path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{PAYLOAD_PREFIX}{id}"))
    }

    pub fn write_payload(&self, id: u64, bytes: &[u8]) -> Result<(), Error> {
        write_private(&self.payload_path(id), bytes)
    }

    pub fn read_payload(&self, id: u64) -> Result<Vec<u8>, Error> {
        fs::read(self.payload_path(id)).map_err(|_| Error::NotFound)
    }

    pub fn remove_payloads(&self, ids: &[u64]) {
        for id in ids {
            let _ = fs::remove_file(self.payload_path(*id));
        }
    }

    /// Whether the user allowed clipboard history. Off until they do.
    pub fn enabled(&self) -> bool {
        fs::read(&self.config)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
            .unwrap_or_default()
            .enabled
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        if let Some(parent) = self.config.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|_| Error::Store)?;
        }
        let bytes = serde_json::to_vec(&Config { enabled }).map_err(|_| Error::Store)?;
        write_private(&self.config, &bytes)
    }
}

/// Write through a 0600 temporary file and rename it into place.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::Store)?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| Error::Store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_clipboard::{summarise, Kind};

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rmac-clipboard-{name}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn directory_and_files_are_private() {
        let root = scratch("private");
        let store = Store::at(root.join("clipboard"), root.join("config/clipboard.json"));
        store.prepare().unwrap();
        let mode = fs::metadata(root.join("clipboard"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        store.write_payload(7, b"hello").unwrap();
        let mode = fs::metadata(store.payload_path(7))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        store.set_enabled(true).unwrap();
        let mode = fs::metadata(root.join("config/clipboard.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn history_is_off_until_allowed() {
        let root = scratch("consent");
        let store = Store::at(root.join("clipboard"), root.join("config/clipboard.json"));
        assert!(!store.enabled());
        store.set_enabled(true).unwrap();
        assert!(store.enabled());
        store.set_enabled(false).unwrap();
        assert!(!store.enabled());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reload_drops_orphans_and_entries_without_payloads() {
        let root = scratch("reload");
        let store = Store::at(root.join("clipboard"), root.join("config/clipboard.json"));
        store.prepare().unwrap();
        let mut history = History::default();
        let kept = history
            .record(summarise(Kind::Text, "text/plain", b"kept").unwrap(), 1)
            .id;
        let missing = history
            .record(summarise(Kind::Text, "text/plain", b"missing").unwrap(), 2)
            .id;
        store.write_payload(kept, b"kept").unwrap();
        store.write_payload(999, b"orphan").unwrap();
        store.save_history(&history).unwrap();

        let loaded = store.load_history();
        assert!(loaded.get(kept).is_some());
        assert!(loaded.get(missing).is_none());
        assert!(!store.payload_path(999).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_symlink_in_place_of_the_directory_is_refused() {
        let root = scratch("symlink");
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), root.join("clipboard")).unwrap();
        let store = Store::at(root.join("clipboard"), root.join("config/clipboard.json"));
        assert_eq!(store.prepare(), Err(Error::Store));
        let _ = fs::remove_dir_all(&root);
    }
}
