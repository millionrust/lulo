//! Versioned, private per-application window geometry persistence.

use std::cmp::Ordering;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 8 * 1024;
const MAX_APP_ID_BYTES: usize = 128;
const MAX_DIMENSION: f64 = 32_768.0;
const MAX_COORDINATE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mode: WindowMode,
}

impl WindowState {
    pub fn checked(x: f64, y: f64, width: f64, height: f64, mode: WindowMode) -> Option<Self> {
        let state = Self {
            x,
            y,
            width,
            height,
            mode,
        };
        state.is_valid().then_some(state)
    }

    /// Fit persisted geometry onto a connected display. The first display is
    /// the preferred fallback (normally the primary display). Existing
    /// placement wins when it still intersects a display; otherwise the window
    /// is centered on the preferred display. The result is wholly on-screen.
    pub fn fit_to_displays(
        self,
        displays: &[DisplayBounds],
        minimum_width: f64,
        minimum_height: f64,
    ) -> Option<Self> {
        if !self.is_valid() || !valid_dimension(minimum_width) || !valid_dimension(minimum_height) {
            return None;
        }
        let valid_displays = displays
            .iter()
            .copied()
            .filter(DisplayBounds::is_valid)
            .collect::<Vec<_>>();
        let preferred = *valid_displays.first()?;
        let target = valid_displays
            .iter()
            .copied()
            .max_by(|left, right| {
                overlap_area(self, *left)
                    .partial_cmp(&overlap_area(self, *right))
                    .unwrap_or(Ordering::Equal)
            })
            .filter(|display| overlap_area(self, *display) > 0.0)
            .unwrap_or(preferred);

        let width = self
            .width
            .clamp(minimum_width.min(target.width), target.width);
        let height = self
            .height
            .clamp(minimum_height.min(target.height), target.height);
        let had_visible_placement = overlap_area(self, target) > 0.0;
        let (x, y) = if had_visible_placement {
            (
                self.x.clamp(target.x, target.right() - width),
                self.y.clamp(target.y, target.bottom() - height),
            )
        } else {
            (
                target.x + (target.width - width) / 2.0,
                target.y + (target.height - height) / 2.0,
            )
        };
        Self::checked(x, y, width, height, self.mode)
    }

    fn is_valid(self) -> bool {
        valid_coordinate(self.x)
            && valid_coordinate(self.y)
            && valid_dimension(self.width)
            && valid_dimension(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl DisplayBounds {
    pub fn checked(x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        let bounds = Self {
            x,
            y,
            width,
            height,
        };
        bounds.is_valid().then_some(bounds)
    }

    fn is_valid(&self) -> bool {
        valid_coordinate(self.x)
            && valid_coordinate(self.y)
            && valid_dimension(self.width)
            && valid_dimension(self.height)
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

fn overlap_area(window: WindowState, display: DisplayBounds) -> f64 {
    let width = (window.x + window.width).min(display.right()) - window.x.max(display.x);
    let height = (window.y + window.height).min(display.bottom()) - window.y.max(display.y);
    width.max(0.0) * height.max(0.0)
}

fn valid_coordinate(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_COORDINATE
}

fn valid_dimension(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_DIMENSION
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolvePath,
    Read,
    Parse,
    Validate,
    CreateDirectory,
    Serialize,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: ErrorKind,
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
            "Window-state operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn from_environment(app_id: &str) -> Result<Self, Error> {
        validate_app_id(app_id)?;
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
            state_home
                .join("rmac/windows")
                .join(format!("{app_id}.json")),
        ))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<WindowState>, Error> {
        match self.read(&self.path) {
            Ok(Some(state)) => Ok(Some(state)),
            Ok(None) => self.read(&self.last_good_path()),
            Err(primary) if recoverable(primary) => match self.read(&self.last_good_path()) {
                Ok(Some(state)) => Ok(Some(state)),
                Ok(None) => Err(primary),
                Err(_) => Err(primary),
            },
            Err(error) => Err(error),
        }
    }

    fn read(&self, path: &Path) -> Result<Option<WindowState>, Error> {
        let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(Operation::Read, error)),
        };
        let stored: StoredState = serde_json::from_slice(&bytes)
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

    pub fn save(&self, state: WindowState) -> Result<(), Error> {
        if !state.is_valid() {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let bytes = serde_json::to_vec(&StoredState {
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
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("window.json");
        self.path.with_file_name(format!("{name}.last-good"))
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

fn validate_app_id(app_id: &str) -> Result<(), Error> {
    let valid = !app_id.is_empty()
        && app_id.len() <= MAX_APP_ID_BYTES
        && app_id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && app_id
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && !app_id.contains("..");
    valid
        .then_some(())
        .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredState {
    version: u32,
    state: WindowState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_path(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rmac-window-state-{name}-{}-{sequence}/state.json",
            std::process::id()
        ))
    }

    #[test]
    fn state_round_trips_through_private_atomic_store() {
        let path = test_path("round-trip");
        let store = Store::at(path.clone());
        let state = WindowState::checked(40.0, 80.0, 900.0, 640.0, WindowMode::Maximized).unwrap();

        assert_eq!(store.load().unwrap(), None);
        store.save(state).unwrap();
        assert_eq!(store.load().unwrap(), Some(state));

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_primary_recovers_the_last_known_good_state() {
        let path = test_path("recovery");
        let store = Store::at(path.clone());
        let state = WindowState::checked(40.0, 80.0, 900.0, 640.0, WindowMode::Windowed).unwrap();
        store.save(state).unwrap();
        std::fs::write(&path, b"not json").unwrap();

        assert_eq!(store.load().unwrap(), Some(state));

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn disconnected_placement_moves_to_primary_and_preserves_mode() {
        let state =
            WindowState::checked(4000.0, 200.0, 1600.0, 1000.0, WindowMode::Maximized).unwrap();
        let primary = DisplayBounds::checked(0.0, 0.0, 1280.0, 720.0).unwrap();

        assert_eq!(
            state.fit_to_displays(&[primary], 640.0, 360.0),
            WindowState::checked(0.0, 0.0, 1280.0, 720.0, WindowMode::Maximized)
        );
    }

    #[test]
    fn visible_placement_is_clamped_wholly_onto_its_display() {
        let state =
            WindowState::checked(1200.0, 650.0, 900.0, 640.0, WindowMode::Windowed).unwrap();
        let primary = DisplayBounds::checked(0.0, 0.0, 1280.0, 720.0).unwrap();

        assert_eq!(
            state.fit_to_displays(&[primary], 640.0, 360.0),
            WindowState::checked(380.0, 80.0, 900.0, 640.0, WindowMode::Windowed)
        );
    }

    #[test]
    fn path_like_application_ids_are_rejected() {
        assert_eq!(
            validate_app_id("../org.rmac.Files").unwrap_err().operation,
            Operation::ResolvePath
        );
        assert!(validate_app_id("org.rmac.Files").is_ok());
    }
}
