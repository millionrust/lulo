//! Volatile wallpaper colour authority shared by the renderer and app themes.

use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

const CURRENT_VERSION: u32 = 1;
const MAX_STATE_BYTES: usize = 64 * 1024;

/// Colour information derived from one output's visible wallpaper source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallpaperColor {
    /// Average of an 8×8 sRGB downsample.
    pub dominant: [u8; 3],
    /// Relative luminance of `dominant`, in the inclusive range 0…1.
    pub luminance: f32,
}

/// One atomic publication from the wallpaper service.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WallpaperColors {
    #[serde(default)]
    pub outputs: BTreeMap<String, WallpaperColor>,
}

impl WallpaperColors {
    /// Current global fallback for app tokens. The state remains per-output so
    /// window-local resolution can replace this deterministic choice later.
    pub fn primary(&self) -> Option<WallpaperColor> {
        self.outputs.values().next().copied()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredWallpaperColors {
    version: u32,
    #[serde(default)]
    colors: WallpaperColors,
}

/// `$XDG_RUNTIME_DIR/rmac/wallpaper-colors.json` authority.
#[derive(Clone, Debug)]
pub struct WallpaperColorStore {
    path: PathBuf,
}

impl WallpaperColorStore {
    pub fn from_environment() -> io::Result<Self> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_RUNTIME_DIR is unavailable for wallpaper colours",
                )
            })?;
        Ok(Self::new(runtime.join("rmac/wallpaper-colors.json")))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<WallpaperColors> {
        let bytes = match FileSystem.read_bounded_no_follow(&self.path, MAX_STATE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(WallpaperColors::default());
            }
            Err(error) => return Err(error),
        };
        let stored: StoredWallpaperColors = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if stored.version != CURRENT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported wallpaper colour version {}; expected {CURRENT_VERSION}",
                    stored.version
                ),
            ));
        }
        if stored
            .colors
            .outputs
            .values()
            .any(|color| !color.luminance.is_finite() || !(0.0..=1.0).contains(&color.luminance))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "wallpaper luminance must be finite and between zero and one",
            ));
        }
        Ok(stored.colors)
    }

    /// Atomically publish changed state. Returns false for an identical value.
    pub fn publish(&self, colors: &WallpaperColors) -> io::Result<bool> {
        if self.load().is_ok_and(|current| current == *colors) {
            return Ok(false);
        }
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "wallpaper colour path has no parent",
            )
        })?;
        FileSystem.create_dir_all_private(parent)?;
        let bytes = serde_json::to_vec_pretty(&StoredWallpaperColors {
            version: CURRENT_VERSION,
            colors: colors.clone(),
        })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        FileSystem.write_atomic_private(&self.path, &bytes)?;
        Ok(true)
    }

    pub fn watch(&self) -> io::Result<WallpaperColorWatcher> {
        use notify::Watcher as _;

        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "wallpaper colour path has no parent",
            )
        })?;
        FileSystem.create_dir_all_private(parent)?;
        let target = self.path.clone();
        let callback_target = target.clone();
        let (sender, receiver) = async_channel::bounded(1);
        let callback_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
                match result {
                    // Reads of the snapshot, including this process's own reload,
                    // raise access events; only writes should trigger a reload.
                    Ok(event)
                        if !event.kind.is_access()
                            && event.paths.iter().any(|path| path == &callback_target) =>
                    {
                        let _ = callback_sender.try_send(());
                    }
                    Ok(_) => {}
                    Err(_) => {
                        let _ = callback_sender.try_send(());
                    }
                }
            })
            .map_err(io::Error::other)?;
        watcher
            .watch(parent, notify::RecursiveMode::NonRecursive)
            .map_err(io::Error::other)?;
        Ok(WallpaperColorWatcher {
            receiver,
            _watcher: watcher,
        })
    }
}

pub struct WallpaperColorWatcher {
    receiver: async_channel::Receiver<()>,
    _watcher: notify::RecommendedWatcher,
}

impl WallpaperColorWatcher {
    pub async fn recv(&self) -> Result<(), async_channel::RecvError> {
        self.receiver.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn store(name: &str) -> (PathBuf, WallpaperColorStore) {
        let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "rmac-wallpaper-colors-{name}-{}-{id}",
            std::process::id()
        ));
        (
            root.clone(),
            WallpaperColorStore::new(root.join("state.json")),
        )
    }

    #[test]
    fn publication_round_trips_and_skips_identical_state() {
        let (root, store) = store("round-trip");
        let colors = WallpaperColors {
            outputs: [(
                "eDP-1".into(),
                WallpaperColor {
                    dominant: [120, 40, 200],
                    luminance: 0.17,
                },
            )]
            .into_iter()
            .collect(),
        };
        assert!(store.publish(&colors).unwrap());
        assert_eq!(store.load().unwrap(), colors);
        assert!(!store.publish(&colors).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_state_is_empty() {
        let (root, store) = store("missing");
        assert_eq!(store.load().unwrap(), WallpaperColors::default());
        let _ = std::fs::remove_dir_all(root);
    }
}
