//! Private, versioned persistence for the last selected Settings pane.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::Context;
use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 4 * 1024;
const MAX_PANE_ID_BYTES: usize = 128;
const SAVE_QUIET_PERIOD: Duration = Duration::from_millis(250);

pub(super) struct NavigationPersistence {
    pending: Arc<Mutex<Option<String>>>,
    wake: async_channel::Sender<()>,
}

impl NavigationPersistence {
    pub(super) fn start<T: 'static>(cx: &Context<T>) -> Self {
        let pending: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (wake, updates) = async_channel::bounded(1);
        let worker_pending = Arc::clone(&pending);
        cx.background_executor()
            .spawn(async move {
                let Ok(store) = NavigationStore::from_environment() else {
                    return;
                };
                while updates.recv().await.is_ok() {
                    async_io::Timer::after(SAVE_QUIET_PERIOD).await;
                    while updates.try_recv().is_ok() {}
                    let pane_id = worker_pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.take());
                    if let Some(pane_id) = pane_id {
                        let _ = store.save(&pane_id);
                    }
                }
            })
            .detach();
        Self { pending, wake }
    }

    pub(super) fn restore() -> Option<String> {
        NavigationStore::from_environment()
            .and_then(|store| store.load())
            .ok()
            .flatten()
    }

    pub(super) fn schedule(&self, pane_id: &str) {
        if !valid_pane_id(pane_id) {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(pane_id.to_owned());
            let _ = self.wake.try_send(());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    ResolvePath,
    Read,
    Parse,
    Validate,
    CreateDirectory,
    Serialize,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Error {
    operation: Operation,
    kind: ErrorKind,
}

impl Error {
    fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: Operation, error: io::Error) -> Self {
        Self::new(operation, ErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Settings navigation operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
struct NavigationStore {
    path: PathBuf,
}

impl NavigationStore {
    fn from_environment() -> Result<Self, Error> {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        Ok(Self::at(
            state_home.join("rmac/system-settings/navigation.json"),
        ))
    }

    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<Option<String>, Error> {
        match self.read(&self.path) {
            Ok(Some(pane_id)) => Ok(Some(pane_id)),
            Ok(None) => self.read(&self.last_good_path()),
            Err(primary) if recoverable(primary) => match self.read(&self.last_good_path()) {
                Ok(Some(pane_id)) => Ok(Some(pane_id)),
                Ok(None) | Err(_) => Err(primary),
            },
            Err(error) => Err(error),
        }
    }

    fn read(&self, path: &Path) -> Result<Option<String>, Error> {
        let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(Operation::Read, error)),
        };
        let stored: StoredNavigation = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        if stored.version != VERSION {
            return Err(Error::new(Operation::Parse, ErrorKind::UnsupportedVersion));
        }
        valid_pane_id(&stored.selected_pane)
            .then_some(Some(stored.selected_pane))
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))
    }

    fn save(&self, pane_id: &str) -> Result<(), Error> {
        if !valid_pane_id(pane_id) {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let bytes = serde_json::to_vec(&StoredNavigation {
            version: VERSION,
            selected_pane: pane_id.to_owned(),
        })
        .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.last_good_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn last_good_path(&self) -> PathBuf {
        self.path.with_file_name("navigation.json.last-good")
    }
}

fn valid_pane_id(pane_id: &str) -> bool {
    !pane_id.is_empty()
        && pane_id.len() <= MAX_PANE_ID_BYTES
        && pane_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && pane_id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && pane_id
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid
            | ErrorKind::UnsupportedVersion
            | ErrorKind::Limit
            | ErrorKind::Io(io::ErrorKind::InvalidData)
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredNavigation {
    version: u32,
    selected_pane: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_path(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rmac-settings-navigation-{name}-{}-{sequence}/navigation.json",
            std::process::id()
        ))
    }

    #[test]
    fn selected_pane_round_trips_and_recovers_from_primary_corruption() {
        let path = test_path("round-trip");
        let store = NavigationStore::at(path.clone());
        assert_eq!(store.load().unwrap(), None);
        store.save("privacy-security").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("privacy-security"));
        std::fs::write(&path, b"corrupt").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("privacy-security"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn malformed_pane_ids_never_reach_storage() {
        let path = test_path("invalid");
        let store = NavigationStore::at(path.clone());
        assert_eq!(
            store.save("../privacy").unwrap_err().kind,
            ErrorKind::Invalid
        );
        assert!(!path.exists());
    }
}
