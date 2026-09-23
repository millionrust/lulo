//! The "already set up" marker that makes the assistant run once:
//! `$XDG_CONFIG_HOME/rmac/setup-assistant-complete` (`~/.config` when the
//! variable is unset or relative, as the XDG spec requires).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::flow::Completion;

pub const MARKER_NAME: &str = "setup-assistant-complete";

/// The marker path for the given `XDG_CONFIG_HOME` and `HOME` values.
pub fn marker_path(config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let config = config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })?;
    Some(config.join("rmac").join(MARKER_NAME))
}

/// The marker path for this process.
pub fn current_marker_path() -> Option<PathBuf> {
    marker_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// The marker's contents: one line naming how the flow ended.
pub fn marker_contents(completion: Completion) -> &'static str {
    match completion {
        Completion::Finished => "finished\n",
        Completion::Skipped => "skipped\n",
    }
}

pub fn is_complete() -> bool {
    current_marker_path().is_some_and(|path| path.exists())
}

pub fn mark_complete(completion: Completion) -> std::io::Result<()> {
    let path = current_marker_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "neither XDG_CONFIG_HOME nor HOME is an absolute path",
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    rmac_storage::atomic_write(&path, marker_contents(completion).as_bytes())
}
