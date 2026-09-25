//! "Return to Previous Size" bookkeeping for keyboard window tiling (WIN-01,
//! Window ▸ Move & Resize): the frame a window had just before 🌐⌃F Fill,
//! 🌐⌃C Centre or a 🌐⌃-arrow half, so 🌐⌃R can put it back.
//!
//! The frame is stored as a percentage of the output's working area (the
//! same space [`crate::TileRegion::frame_percent`] measures), not absolute
//! points, so restoring it is exactly [`crate::Action::SetWindowFrame`] --
//! the same floating-frame mechanism Fill, Centre and the halves already use
//! -- with no extra compositor capability to add. It persists to
//! `$XDG_RUNTIME_DIR/rmac/tile-history.json` so the shortcut's one-shot CLI
//! process remembers it between key presses.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::WindowId;

/// A window's frame as a percentage of its output's working area (0-100),
/// the same convention as [`crate::TileRegion::frame_percent`].
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct FramePercent {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Convert a window's frame, and the output's size, from absolute
/// output-local logical points into [`FramePercent`] -- the percentage of
/// the working area below `top_inset` (the menu bar) and above the bottom
/// `bottom_inset` (the Dock). `None` when the working area has no positive
/// size.
// Two plain rectangles (output and frame) plus the two insets read most
// clearly as named scalars at the call sites.
#[allow(clippy::too_many_arguments)]
pub fn frame_to_percent(
    output_width: f64,
    output_height: f64,
    top_inset: f64,
    bottom_inset: f64,
    frame_x: f64,
    frame_y: f64,
    frame_width: f64,
    frame_height: f64,
) -> Option<FramePercent> {
    let work_height = output_height - top_inset - bottom_inset;
    if !(output_width > 0.0 && work_height > 0.0) {
        return None;
    }
    Some(FramePercent {
        x: (frame_x / output_width) * 100.0,
        y: ((frame_y - top_inset) / work_height) * 100.0,
        width: (frame_width / output_width) * 100.0,
        height: (frame_height / work_height) * 100.0,
    })
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TileHistoryStore {
    previous: Vec<(WindowId, FramePercent)>,
}

impl TileHistoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn previous(&self, window: WindowId) -> Option<FramePercent> {
        self.previous
            .iter()
            .find(|(id, _)| *id == window)
            .map(|(_, frame)| *frame)
    }

    /// Remember `frame` as where `window` was before a tiling shortcut,
    /// unless one is already recorded: repeated Fill, Centre or halves keep
    /// the frame from before the *first* of them, so 🌐⌃R always lands back
    /// on the window's original size, not on its last tiled one.
    pub fn record(&mut self, window: WindowId, frame: FramePercent) {
        if self.previous(window).is_none() {
            self.previous.push((window, frame));
        }
    }

    /// Forget and return the recorded frame. 🌐⌃R calls this once, so a
    /// second press does not bounce back to the size it just restored.
    pub fn take(&mut self, window: WindowId) -> Option<FramePercent> {
        let index = self.previous.iter().position(|(id, _)| *id == window)?;
        Some(self.previous.remove(index).1)
    }

    /// Drop entries for windows that no longer exist, so a closed window's
    /// id is never reused for another window's frame.
    pub fn prune(&mut self, live: &[WindowId]) {
        self.previous.retain(|(id, _)| live.contains(id));
    }

    /// Read the store, treating a missing or unreadable file as empty. The
    /// history is a convenience cache, never a source of truth.
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
        Some(
            PathBuf::from(runtime)
                .join("rmac")
                .join("tile-history.json"),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_to_percent_reads_the_working_area_below_the_bar_and_above_the_dock() {
        // The reference PC: 1536 x 864, 29 pt menu bar, 89 pt Dock
        // reservation (WIN-02). A window filling the whole working area.
        let percent = frame_to_percent(1536.0, 864.0, 29.0, 89.0, 0.0, 29.0, 1536.0, 746.0)
            .expect("positive working area");
        assert!((percent.x - 0.0).abs() < 1e-9);
        assert!((percent.y - 0.0).abs() < 1e-9);
        assert!((percent.width - 100.0).abs() < 1e-9);
        assert!((percent.height - 100.0).abs() < 1e-9);

        // The right half, as `TileRegion::Right` would set it: x=50%,
        // width=50%, full height.
        let right = frame_to_percent(1536.0, 864.0, 29.0, 89.0, 768.0, 29.0, 768.0, 746.0)
            .expect("positive working area");
        assert!((right.x - 50.0).abs() < 1e-9);
        assert!((right.width - 50.0).abs() < 1e-9);

        // A screen with no room between the bar and the Dock has no working
        // area to measure a percentage against.
        assert_eq!(
            frame_to_percent(1536.0, 100.0, 29.0, 89.0, 0.0, 0.0, 1.0, 1.0),
            None
        );
    }

    #[test]
    fn store_keeps_the_frame_from_before_the_first_tiling_action() {
        let mut store = TileHistoryStore::new();
        let frame = FramePercent {
            x: 12.0,
            y: 8.0,
            width: 40.0,
            height: 60.0,
        };
        store.record(WindowId(1), frame);
        // A second tiling action (e.g. Centre after Fill) does not overwrite
        // the original frame.
        store.record(
            WindowId(1),
            FramePercent {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        assert_eq!(store.previous(WindowId(1)), Some(frame));

        // Taken once, it is gone.
        assert_eq!(store.take(WindowId(1)), Some(frame));
        assert_eq!(store.take(WindowId(1)), None);
    }

    #[test]
    fn prune_drops_windows_the_compositor_no_longer_reports() {
        let mut store = TileHistoryStore::new();
        let frame = FramePercent {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        };
        store.record(WindowId(1), frame);
        store.record(WindowId(2), frame);
        store.prune(&[WindowId(2)]);
        assert_eq!(store.previous(WindowId(1)), None);
        assert_eq!(store.previous(WindowId(2)), Some(frame));
    }

    #[test]
    fn store_round_trips_through_json() {
        let mut store = TileHistoryStore::new();
        store.record(
            WindowId(9),
            FramePercent {
                x: 1.5,
                y: 2.5,
                width: 3.5,
                height: 4.5,
            },
        );
        let directory = std::env::temp_dir().join(format!(
            "rmac-tile-history-test-{}-{:?}",
            std::process::id(),
            std::time::Instant::now()
        ));
        let path = directory.join("tile-history.json");
        store.save(&path).unwrap();
        let loaded = TileHistoryStore::load(&path);
        assert_eq!(loaded, store);
        let _ = fs::remove_dir_all(&directory);
    }
}
