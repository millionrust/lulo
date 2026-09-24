//! Private, versioned Finder presentation and safe tab-session continuity.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Context, Window};
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

/// Every open window saves to its own file under `windows/`, so two windows
/// changing folders never race to overwrite each other's tabs. The single
/// shared `presentation.json` this module also keeps is not a live window's
/// file at all: it only ever changes at the moment a window actually closes
/// ([`FinderPersistence::close`]), so it always holds the *last-closed*
/// window's state for the next launch to restore, as the Mac's window
/// restoration does.
pub(super) struct FinderPersistence {
    window_id: String,
    pending: Arc<Mutex<Option<FinderState>>>,
    wake: async_channel::Sender<()>,
}

impl FinderPersistence {
    pub(super) fn start<T: 'static>(window_id: String, cx: &Context<T>) -> Self {
        let pending: Arc<Mutex<Option<FinderState>>> = Arc::new(Mutex::new(None));
        let (wake, updates) = async_channel::bounded(1);
        let worker_pending = Arc::clone(&pending);
        let worker_window_id = window_id.clone();
        cx.background_executor()
            .spawn(async move {
                let Ok(store) = FinderStateStore::for_window(&worker_window_id) else {
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
                        if let Err(error) = store.save(&state) {
                            eprintln!("could not save the Files window state: {error}");
                        }
                    }
                }
            })
            .detach();
        Self {
            window_id,
            pending,
            wake,
        }
    }

    /// The last-closed window's state, for a launch that shows the default
    /// window (not one opened at an explicit destination, as ⌘N's is).
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

    /// This window is closing: `state` becomes the state a fresh launch
    /// restores, and this window's now-unneeded per-window file is removed.
    /// Runs synchronously (there is no window left to keep spawning tasks
    /// against by the time this returns).
    pub(super) fn close(&self, state: Option<FinderState>) {
        if let Some(state) = state.filter(|state| state.is_valid()) {
            match FinderStateStore::from_environment() {
                Ok(store) => {
                    if let Err(error) = store.save(&state) {
                        eprintln!("could not save the closed Files window's state: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("could not save the closed Files window's state: {error}");
                }
            }
        }
        if let Ok(store) = FinderStateStore::for_window(&self.window_id) {
            store.remove();
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
        self.select_first_for_gallery(cx);
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
        if let Some(state) = self.finder_state() {
            self.finder_persistence.schedule(state);
        }
    }

    fn finder_state(&self) -> Option<FinderState> {
        let presentation =
            PresentationState::checked(self.view, self.sidebar_visible, self.sidebar_width)?;
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
        FinderState::checked(presentation, tabs, self.active)
    }

    /// ⌘W with one tab, the traffic-light close button, and any close
    /// request from outside the window (Quit, logging out) all end here:
    /// this window's state becomes the one a fresh launch restores, and the
    /// window itself closes. A window never gets a second chance to save
    /// after this runs, so the save happens before `remove_window`, not
    /// after.
    pub(super) fn close_finder_window(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.finder_persistence.close(self.finder_state());
        window.remove_window();
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
    fn state_home() -> Result<PathBuf, Error> {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))
    }

    /// The single file a fresh launch restores: only ever written at the
    /// moment a window closes, never while windows are live.
    fn from_environment() -> Result<Self, Error> {
        Ok(Self::at(
            Self::state_home()?.join("rmac/files/presentation.json"),
        ))
    }

    /// One live window's own file. `window_id` is generated in-process
    /// (digits and dashes only), so it never escapes `windows/`.
    fn for_window(window_id: &str) -> Result<Self, Error> {
        Ok(Self::at(
            Self::state_home()?
                .join("rmac/files/windows")
                .join(format!("{window_id}.json")),
        ))
    }

    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Removes this store's file and its `.last-good` companion. Best
    /// effort: a leftover per-window file from a crash is never read back,
    /// so failing to remove it costs only disk space, not correctness.
    fn remove(&self) {
        for path in [self.path.clone(), self.last_good_path()] {
            if let Err(error) = std::fs::remove_file(&path) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!("could not remove a closed Files window's state file: {error}");
                }
            }
        }
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

    /// Derived from this store's own file name, not a fixed one: every
    /// per-window store under `windows/` needs its own `.last-good`
    /// companion, not one they'd all collide on.
    fn last_good_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map_or_else(OsString::new, OsStr::to_os_string);
        name.push(".last-good");
        self.path.with_file_name(name)
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

    #[test]
    fn removing_a_store_clears_its_state_file_and_last_good_backup() {
        let path = test_path("remove");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let store = FinderStateStore::at(path.clone());
        let state = FinderState::checked(
            PresentationState::checked(ViewMode::List, true, 200.0).unwrap(),
            vec![PathBuf::from("/tmp")],
            0,
        )
        .unwrap();
        store.save(&state).unwrap();
        assert!(path.exists());
        assert!(store.last_good_path().exists());

        store.remove();

        assert!(!path.exists());
        assert!(!store.last_good_path().exists());
        // A store with nothing left to remove is a silent no-op, as it is
        // for a per-window file that never got as far as its first save.
        store.remove();

        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn each_store_keeps_its_own_last_good_backup_name() {
        // Two windows' stores share a directory but must never collide on
        // one `.last-good` file, or the second window's save would corrupt
        // the first window's recovery copy.
        let a = FinderStateStore::at(PathBuf::from("/tmp/rmac-files-windows/111.json"));
        let b = FinderStateStore::at(PathBuf::from("/tmp/rmac-files-windows/222.json"));

        assert_ne!(a.last_good_path(), b.last_good_path());
        assert_eq!(
            a.last_good_path(),
            PathBuf::from("/tmp/rmac-files-windows/111.json.last-good")
        );
    }
}
