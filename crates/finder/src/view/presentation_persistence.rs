//! Private, versioned Finder presentation and safe tab-session continuity.

use std::fmt;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::Context;
use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

use super::{FinderView, ViewMode};

pub(super) const MIN_SIDEBAR_WIDTH: f32 = 140.0;
pub(super) const MAX_SIDEBAR_WIDTH: f32 = 360.0;
pub(super) const MAX_RESTORED_TABS: usize = 16;
// design-lab/finder.html: an 8 pt inset plus the 148 pt floating panel.
const DEFAULT_SIDEBAR_WIDTH: f32 = 156.0;
const LEGACY_DEFAULT_SIDEBAR_WIDTH: f32 = 220.0;
/// The opaque-column default used before the Tahoe floating panel.
const PREVIOUS_DEFAULT_SIDEBAR_WIDTH: f32 = 180.0;
const CURRENT_VERSION: u32 = 3;
const MAX_FILE_BYTES: usize = 80 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct FinderState {
    pub(super) presentation: PresentationState,
    pub(super) tabs: Vec<PathBuf>,
    pub(super) active_tab: usize,
}

impl FinderState {
    pub(super) fn checked(
        presentation: PresentationState,
        tabs: Vec<PathBuf>,
        active_tab: usize,
    ) -> Option<Self> {
        let state = Self {
            presentation,
            tabs,
            active_tab,
        };
        state.is_valid().then_some(state)
    }

    pub(super) fn restorable_session(&self, home: &Path) -> (Vec<PathBuf>, usize) {
        let mut restored = Vec::new();
        let mut restored_active = None;
        for (index, path) in self.tabs.iter().enumerate() {
            if !path.is_dir() {
                continue;
            }
            if index == self.active_tab {
                restored_active = Some(restored.len());
            }
            restored.push(path.clone());
        }
        if restored.is_empty() {
            return (vec![home.to_path_buf()], 0);
        }
        (restored, restored_active.unwrap_or(0))
    }

    fn is_valid(&self) -> bool {
        self.presentation.is_valid()
            && self.tabs.len() <= MAX_RESTORED_TABS
            && if self.tabs.is_empty() {
                self.active_tab == 0
            } else {
                self.active_tab < self.tabs.len()
            }
            && self.tabs.iter().all(|path| {
                path.is_absolute()
                    && path.to_str().is_some()
                    && path.as_os_str().as_bytes().len() <= MAX_PATH_BYTES
            })
    }
}

pub(super) struct FinderPersistence {
    pending: Arc<Mutex<Option<FinderState>>>,
    wake: async_channel::Sender<()>,
}

impl FinderPersistence {
    pub(super) fn start<T: 'static>(cx: &Context<T>) -> Self {
        let pending: Arc<Mutex<Option<FinderState>>> = Arc::new(Mutex::new(None));
        let (wake, updates) = async_channel::bounded(1);
        let worker_pending = Arc::clone(&pending);
        cx.background_executor()
            .spawn(async move {
                let Ok(store) = FinderStateStore::from_environment() else {
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
                        let _ = store.save(&state);
                    }
                }
            })
            .detach();
        Self { pending, wake }
    }

    pub(super) fn restore() -> FinderState {
        FinderStateStore::from_environment()
            .and_then(|store| store.load())
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub(super) fn schedule(&self, state: FinderState) {
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
        if mode != ViewMode::Column {
            self.column_selection = None;
        }
        self.operation_error = None;
        self.persist_finder_state();
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.resizing_sidebar = false;
        self.persist_finder_state();
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
        self.persist_finder_state();
        cx.notify();
    }

    pub(super) fn finish_sidebar_resize(&mut self) {
        self.resizing_sidebar = false;
    }

    pub(super) fn persist_finder_state(&self) {
        let Some(presentation) =
            PresentationState::checked(self.view, self.sidebar_visible, self.sidebar_width)
        else {
            return;
        };
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                if index == self.active {
                    self.cwd.clone()
                } else {
                    tab.cwd.clone()
                }
            })
            .collect();
        if let Some(state) = FinderState::checked(presentation, tabs, self.active) {
            self.finder_persistence.schedule(state);
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
            "Finder state operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
struct FinderStateStore {
    path: PathBuf,
}

impl FinderStateStore {
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

    fn load(&self) -> Result<Option<FinderState>, Error> {
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

    fn read(&self, path: &Path) -> Result<Option<FinderState>, Error> {
        let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(Operation::Read, error)),
        };
        let version: VersionProbe = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        let state = match version.version {
            1 => {
                let stored: StoredVersion1 = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
                FinderState::checked(stored.state, Vec::new(), 0)
            }
            2 => {
                let stored: StoredVersion2 = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
                let mut state = stored.state;
                if state.presentation.sidebar_width == LEGACY_DEFAULT_SIDEBAR_WIDTH
                    || state.presentation.sidebar_width == PREVIOUS_DEFAULT_SIDEBAR_WIDTH
                {
                    state.presentation.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
                }
                state.is_valid().then_some(state)
            }
            CURRENT_VERSION => {
                let stored: StoredVersion3 = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
                let mut state = stored.state;
                if state.presentation.sidebar_width == PREVIOUS_DEFAULT_SIDEBAR_WIDTH {
                    state.presentation.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
                }
                state.is_valid().then_some(state)
            }
            _ => {
                return Err(Error::new(Operation::Parse, ErrorKind::UnsupportedVersion));
            }
        };
        state
            .map(Some)
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))
    }

    fn save(&self, state: &FinderState) -> Result<(), Error> {
        if !state.is_valid() {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let bytes = serde_json::to_vec(&StoredVersion3 {
            version: CURRENT_VERSION,
            state: state.clone(),
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

#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredVersion1 {
    version: u32,
    state: PresentationState,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredVersion2 {
    version: u32,
    state: FinderState,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredVersion3 {
    version: u32,
    state: FinderState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_path(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rmac-finder-state-{name}-{}-{sequence}/presentation.json",
            std::process::id()
        ))
    }

    #[test]
    fn default_sidebar_uses_the_measured_compact_width() {
        assert_eq!(PresentationState::default().sidebar_width, 156.0);
    }

    #[test]
    fn state_round_trips_and_recovers_from_primary_corruption() {
        let path = test_path("round-trip");
        let store = FinderStateStore::at(path.clone());
        let presentation = PresentationState::checked(ViewMode::Gallery, false, 248.0).unwrap();
        let state = FinderState::checked(presentation, vec![PathBuf::from("/tmp")], 0).unwrap();
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap(), Some(state.clone()));
        std::fs::write(&path, b"corrupt").unwrap();
        assert_eq!(store.load().unwrap(), Some(state));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn version_one_presentation_migrates_without_unsafe_tabs() {
        let path = test_path("migration");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let presentation = PresentationState::checked(ViewMode::Icon, true, 220.0).unwrap();
        let bytes = serde_json::to_vec(&StoredVersion1 {
            version: 1,
            state: presentation,
        })
        .unwrap();
        std::fs::write(&path, bytes).unwrap();

        assert_eq!(
            FinderStateStore::at(path.clone()).load().unwrap(),
            Some(FinderState {
                presentation,
                tabs: Vec::new(),
                active_tab: 0,
            })
        );
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn version_two_migrates_only_the_old_default_sidebar_width() {
        let path = test_path("default-sidebar-width-migration");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let state = FinderState::checked(
            PresentationState::checked(ViewMode::Icon, true, 220.0).unwrap(),
            vec![PathBuf::from("/tmp")],
            0,
        )
        .unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&StoredVersion2 { version: 2, state }).unwrap(),
        )
        .unwrap();

        let migrated = FinderStateStore::at(path.clone()).load().unwrap().unwrap();
        assert_eq!(migrated.presentation.sidebar_width, 156.0);

        std::fs::remove_dir_all(parent).unwrap();

        let path = test_path("custom-sidebar-width-migration");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let state = FinderState::checked(
            PresentationState::checked(ViewMode::Icon, true, 248.0).unwrap(),
            vec![PathBuf::from("/tmp")],
            0,
        )
        .unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&StoredVersion2 { version: 2, state }).unwrap(),
        )
        .unwrap();

        let migrated = FinderStateStore::at(path.clone()).load().unwrap().unwrap();
        assert_eq!(migrated.presentation.sidebar_width, 248.0);

        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn missing_tabs_are_dropped_and_sidebar_width_is_bounded() {
        let presentation = PresentationState::checked(ViewMode::List, true, 900.0).unwrap();
        assert_eq!(presentation.sidebar_width, MAX_SIDEBAR_WIDTH);
        let home = std::env::temp_dir();
        let missing = home.join("rmac-definitely-missing-tab");
        let state = FinderState::checked(presentation, vec![home.clone(), missing], 1).unwrap();

        assert_eq!(state.restorable_session(&home), (vec![home], 0));
    }
}
