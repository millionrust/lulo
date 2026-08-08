//! Private, versioned Finder view and sidebar continuity.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::Context;
use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

use super::{FinderView, ViewMode};

pub(super) const MIN_SIDEBAR_WIDTH: f32 = 160.0;
pub(super) const MAX_SIDEBAR_WIDTH: f32 = 360.0;
const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;
const VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 4 * 1024;
const SAVE_QUIET_PERIOD: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct PresentationState {
    pub(super) view: ViewMode,
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_width: f32,
}

impl Default for PresentationState {
    fn default() -> Self {
        Self {
            view: ViewMode::List,
            sidebar_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl PresentationState {
    pub(super) fn checked(
        view: ViewMode,
        sidebar_visible: bool,
        sidebar_width: f32,
    ) -> Option<Self> {
        sidebar_width.is_finite().then_some(Self {
            view,
            sidebar_visible,
            sidebar_width: sidebar_width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
        })
    }

    fn is_valid(self) -> bool {
        self.sidebar_width.is_finite()
            && (MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH).contains(&self.sidebar_width)
    }
}

pub(super) struct PresentationPersistence {
    pending: Arc<Mutex<Option<PresentationState>>>,
    wake: async_channel::Sender<()>,
}

impl PresentationPersistence {
    pub(super) fn start<T: 'static>(cx: &Context<T>) -> Self {
        let pending: Arc<Mutex<Option<PresentationState>>> = Arc::new(Mutex::new(None));
        let (wake, updates) = async_channel::bounded(1);
        let worker_pending = Arc::clone(&pending);
        cx.background_executor()
            .spawn(async move {
                let Ok(store) = PresentationStore::from_environment() else {
                    return;
                };
                while updates.recv().await.is_ok() {
                    async_io::Timer::after(SAVE_QUIET_PERIOD).await;
                    while updates.try_recv().is_ok() {}
                    let state = worker_pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.take());
                    if let Some(state) = state {
                        let _ = store.save(state);
                    }
                }
            })
            .detach();
        Self { pending, wake }
    }

    pub(super) fn restore() -> PresentationState {
        PresentationStore::from_environment()
            .and_then(|store| store.load())
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub(super) fn schedule(&self, state: PresentationState) {
        if !state.is_valid() {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(state);
            let _ = self.wake.try_send(());
        }
    }
}

impl FinderView {
    pub(super) fn select_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.trash_view && mode == ViewMode::Column {
            self.operation_error = Some("Column view is unavailable in Trash".into());
            cx.notify();
            return;
        }
        self.view = mode;
        self.persist_presentation();
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.resizing_sidebar = false;
        self.persist_presentation();
        cx.notify();
    }

    pub(super) fn begin_sidebar_resize(&mut self) {
        self.resizing_sidebar = true;
    }

    pub(super) fn resize_sidebar(&mut self, width: f32, cx: &mut Context<Self>) {
        if !self.resizing_sidebar || !width.is_finite() {
            return;
        }
        self.sidebar_width = width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        self.persist_presentation();
        cx.notify();
    }

    pub(super) fn finish_sidebar_resize(&mut self) {
        self.resizing_sidebar = false;
    }

    fn persist_presentation(&self) {
        if let Some(state) =
            PresentationState::checked(self.view, self.sidebar_visible, self.sidebar_width)
        {
            self.presentation_persistence.schedule(state);
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
            "Finder presentation operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
struct PresentationStore {
    path: PathBuf,
}

impl PresentationStore {
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
        Ok(Self::at(state_home.join("rmac/files/presentation.json")))
    }

    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<Option<PresentationState>, Error> {
        match self.read(&self.path) {
            Ok(Some(state)) => Ok(Some(state)),
            Ok(None) => self.read(&self.last_good_path()),
            Err(primary) if recoverable(primary) => match self.read(&self.last_good_path()) {
                Ok(Some(state)) => Ok(Some(state)),
                Ok(None) | Err(_) => Err(primary),
            },
            Err(error) => Err(error),
        }
    }

    fn read(&self, path: &Path) -> Result<Option<PresentationState>, Error> {
        let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(Operation::Read, error)),
        };
        let stored: StoredPresentation = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        if stored.version != VERSION {
            return Err(Error::new(Operation::Parse, ErrorKind::UnsupportedVersion));
        }
        stored
            .state
            .is_valid()
            .then_some(Some(stored.state))
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))
    }

    fn save(&self, state: PresentationState) -> Result<(), Error> {
        if !state.is_valid() {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let bytes = serde_json::to_vec(&StoredPresentation {
            version: VERSION,
            state,
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
        self.path.with_file_name("presentation.json.last-good")
    }
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
struct StoredPresentation {
    version: u32,
    state: PresentationState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_path(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rmac-finder-presentation-{name}-{}-{sequence}/presentation.json",
            std::process::id()
        ))
    }

    #[test]
    fn presentation_round_trips_and_recovers_from_primary_corruption() {
        let path = test_path("round-trip");
        let store = PresentationStore::at(path.clone());
        let state = PresentationState::checked(ViewMode::Gallery, false, 248.0).unwrap();
        store.save(state).unwrap();
        assert_eq!(store.load().unwrap(), Some(state));
        std::fs::write(&path, b"corrupt").unwrap();
        assert_eq!(store.load().unwrap(), Some(state));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn restored_sidebar_width_is_bounded() {
        assert_eq!(
            PresentationState::checked(ViewMode::Icon, true, 10.0)
                .unwrap()
                .sidebar_width,
            MIN_SIDEBAR_WIDTH
        );
        assert_eq!(
            PresentationState::checked(ViewMode::List, true, 900.0)
                .unwrap()
                .sidebar_width,
            MAX_SIDEBAR_WIDTH
        );
    }
}
