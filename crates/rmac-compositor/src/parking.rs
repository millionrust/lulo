//! Parked-window bookkeeping for the compositor-neutral minimize/hide model.
//!
//! niri has no native minimize, so §2.2 parks a window on the hidden
//! `rmac-parking` workspace. Restoring it needs the workspace it came from,
//! which the shell records here and persists to
//! `$XDG_RUNTIME_DIR/rmac/parking.json` so the menu bar and the Dock agree on
//! what is hidden.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{window_is_parked, Action, Snapshot, WindowId, WorkspaceId, PARKING_WORKSPACE};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParkedWindow {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    /// App id and title let the Dock label a minimized tile (§4.11); they are
    /// `None` for older store files and for windows hidden without a tile.
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Snapshot captured just before the window was parked, shown as the
    /// tile's thumbnail. Absolute path into the session's runtime directory.
    #[serde(default)]
    pub thumbnail: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParkingStore {
    parked: Vec<ParkedWindow>,
}

impl ParkingStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[ParkedWindow] {
        &self.parked
    }

    pub fn is_empty(&self) -> bool {
        self.parked.is_empty()
    }

    pub fn origin(&self, window: WindowId) -> Option<WorkspaceId> {
        self.parked
            .iter()
            .find(|entry| entry.window == window)
            .map(|entry| entry.workspace)
    }

    /// Remember where `window` came from. The first recorded origin wins so a
    /// failed restore that re-parks cannot lose the real workspace.
    pub fn record(&mut self, window: WindowId, workspace: WorkspaceId) {
        if self.origin(window).is_none() {
            self.parked.push(ParkedWindow {
                window,
                workspace,
                app_id: None,
                title: None,
                thumbnail: None,
            });
        }
    }

    pub fn forget(&mut self, window: WindowId) -> Option<WorkspaceId> {
        let index = self
            .parked
            .iter()
            .position(|entry| entry.window == window)?;
        Some(self.parked.remove(index).workspace)
    }

    pub fn entry(&self, window: WindowId) -> Option<&ParkedWindow> {
        self.parked.iter().find(|entry| entry.window == window)
    }

    /// Attach the thumbnail captured before parking. Returns whether an entry
    /// existed to attach it to.
    pub fn set_thumbnail(&mut self, window: WindowId, thumbnail: PathBuf) -> bool {
        match self.parked.iter_mut().find(|entry| entry.window == window) {
            Some(entry) => {
                entry.thumbnail = Some(thumbnail);
                true
            }
            None => false,
        }
    }

    /// Record the origin workspace of every id that still sits on a normal
    /// workspace of `snapshot`; windows already parked keep their origin.
    pub fn record_from(&mut self, snapshot: &Snapshot, windows: &[WindowId]) {
        for window in windows {
            let Some(candidate) = snapshot
                .windows
                .iter()
                .find(|candidate| candidate.id == *window)
            else {
                continue;
            };
            let Some(workspace) = candidate.workspace else {
                continue;
            };
            if workspace_name(snapshot, workspace).as_deref() == Some(PARKING_WORKSPACE) {
                continue;
            }
            if self.origin(*window).is_none() {
                self.parked.push(ParkedWindow {
                    window: *window,
                    workspace,
                    app_id: candidate.app_id.clone(),
                    title: candidate.title.clone(),
                    thumbnail: None,
                });
            }
        }
    }

    /// Drop entries whose window is gone from the compositor or no longer
    /// parked, so a niri restart cannot restore a stale window id. Returns
    /// the dropped entries so a caller can delete their thumbnails.
    pub fn prune(&mut self, snapshot: &Snapshot) -> Vec<ParkedWindow> {
        let (kept, dropped) =
            std::mem::take(&mut self.parked)
                .into_iter()
                .partition(|entry: &ParkedWindow| {
                    snapshot.windows.iter().any(|window| {
                        window.id == entry.window && window_is_parked(snapshot, window)
                    })
                });
        self.parked = kept;
        dropped
    }

    /// Forget and expand restore actions for the ids that were parked here.
    pub fn restore_actions(&mut self, windows: &[WindowId]) -> Vec<Action> {
        windows
            .iter()
            .filter_map(|window| {
                self.forget(*window).map(|workspace| Action::RestoreWindow {
                    window: *window,
                    workspace,
                })
            })
            .collect()
    }

    /// Read the store, treating a missing or unreadable file as empty. The
    /// parking set is a convenience cache, never a source of truth.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let encoded = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&temporary, encoded)?;
        fs::rename(&temporary, path)
    }

    pub fn default_path() -> Option<PathBuf> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        Some(PathBuf::from(runtime).join("rmac").join("parking.json"))
    }

    /// The directory minimized-window thumbnails are cached in, next to the
    /// store so the runtime directory owns the lifetime of both.
    pub fn default_thumbnail_dir() -> Option<PathBuf> {
        Self::default_path().map(|store| Self::thumbnail_dir_beside(&store))
    }

    /// [`Self::default_thumbnail_dir`] for an explicit store path.
    pub fn thumbnail_dir_beside(store_path: &Path) -> PathBuf {
        store_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("thumbnails")
    }

    /// The file one capture of `window` goes to: `<window>-<stamp>.png`.
    /// Every capture gets its own name because the Dock's image cache is
    /// keyed by path, so reusing a name would show the previous picture the
    /// next time the same window is minimized.
    pub fn thumbnail_path_in(dir: &Path, window: WindowId, stamp: u128) -> PathBuf {
        dir.join(format!("{}-{stamp}.png", window.0))
    }

    /// The newest thumbnail captured for `window` in the default directory.
    pub fn current_thumbnail(window: WindowId) -> Option<PathBuf> {
        Self::newest_thumbnail_in(&Self::default_thumbnail_dir()?, window)
    }

    /// The newest `<window>-<stamp>.png` in `dir`.
    pub fn newest_thumbnail_in(dir: &Path, window: WindowId) -> Option<PathBuf> {
        thumbnail_files(dir)
            .into_iter()
            .filter(|(owner, _, _)| *owner == window)
            .max_by_key(|(_, stamp, _)| *stamp)
            .map(|(_, _, path)| path)
    }

    /// Delete every thumbnail of `window` in `dir`. Missing files are fine.
    pub fn remove_thumbnails_in(dir: &Path, window: WindowId) {
        Self::sweep_thumbnails_in(dir, |owner| owner != window);
    }

    /// Delete every thumbnail in `dir` whose window `keep` rejects, such as
    /// windows that no longer exist after a crash or a niri restart.
    pub fn sweep_thumbnails_in(dir: &Path, keep: impl Fn(WindowId) -> bool) {
        for (owner, _, path) in thumbnail_files(dir) {
            if !keep(owner) {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub fn load_default() -> Self {
        Self::default_path()
            .map(|path| Self::load(&path))
            .unwrap_or_default()
    }

    pub fn save_default(&self) -> io::Result<()> {
        match Self::default_path() {
            Some(path) => self.save(&path),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is not set",
            )),
        }
    }
}

/// `(window, stamp, path)` for every `<window>-<stamp>.png` in `dir`.
fn thumbnail_files(dir: &Path) -> Vec<(WindowId, u128, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let (window, stamp) = parse_thumbnail_name(name.to_str()?)?;
            Some((window, stamp, entry.path()))
        })
        .collect()
}

pub(crate) fn parse_thumbnail_name(name: &str) -> Option<(WindowId, u128)> {
    let stem = name.strip_suffix(".png")?;
    let (window, stamp) = stem.split_once('-')?;
    Some((WindowId(window.parse().ok()?), stamp.parse().ok()?))
}

fn workspace_name(snapshot: &Snapshot, workspace: WorkspaceId) -> Option<String> {
    snapshot
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace)
        .and_then(|candidate| candidate.name.clone())
}
