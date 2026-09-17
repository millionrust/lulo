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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParkedWindow {
    pub window: WindowId,
    pub workspace: WorkspaceId,
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
            self.parked.push(ParkedWindow { window, workspace });
        }
    }

    pub fn forget(&mut self, window: WindowId) -> Option<WorkspaceId> {
        let index = self
            .parked
            .iter()
            .position(|entry| entry.window == window)?;
        Some(self.parked.remove(index).workspace)
    }

    /// Record the origin workspace of every id that still sits on a normal
    /// workspace of `snapshot`; windows already parked keep their origin.
    pub fn record_from(&mut self, snapshot: &Snapshot, windows: &[WindowId]) {
        for window in windows {
            let Some(workspace) = snapshot
                .windows
                .iter()
                .find(|candidate| candidate.id == *window)
                .and_then(|candidate| candidate.workspace)
            else {
                continue;
            };
            if workspace_name(snapshot, workspace).as_deref() == Some(PARKING_WORKSPACE) {
                continue;
            }
            self.record(*window, workspace);
        }
    }

    /// Drop entries whose window is gone from the compositor or no longer
    /// parked, so a niri restart cannot restore a stale window id.
    pub fn prune(&mut self, snapshot: &Snapshot) {
        self.parked.retain(|entry| {
            snapshot
                .windows
                .iter()
                .any(|window| window.id == entry.window && window_is_parked(snapshot, window))
        });
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

fn workspace_name(snapshot: &Snapshot, workspace: WorkspaceId) -> Option<String> {
    snapshot
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace)
        .and_then(|candidate| candidate.name.clone())
}
