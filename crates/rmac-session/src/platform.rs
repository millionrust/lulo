use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rmac_storage::Failure;

use crate::{Error, Operation};

pub trait CommandRunner {
    fn output(&self, program: &str, arguments: &[&str]) -> io::Result<Output>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn output(&self, program: &str, arguments: &[&str]) -> io::Result<Output> {
        Command::new(program).args(arguments).output()
    }
}

#[derive(Clone, Debug)]
pub struct StatePaths {
    /// Persistent marker written when an essential component exhausts its
    /// restart budget. The next login consumes it.
    pub safe_mode: PathBuf,
    /// What the most recent login did with the marker, kept for diagnosis.
    pub last_safe_mode: PathBuf,
    /// Present only while the current login runs in safe mode.
    pub safe_login: PathBuf,
    pub health: PathBuf,
    /// Directory holding the supervisor and every component executable.
    pub libexec: PathBuf,
}

impl StatePaths {
    pub fn from_environment() -> Result<Self, Error> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Failure::message(
                Operation::ResolvePath,
                Path::new("session"),
                "HOME is not set",
            )
        })?;
        let state_root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".local/state"));
        let runtime_root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                Failure::message(
                    Operation::ResolvePath,
                    Path::new("session-health.json"),
                    "XDG_RUNTIME_DIR is not set to an absolute path",
                )
            })?;
        // Package units run /usr/libexec/rmac/*, development units
        // ~/.local/libexec/rmac/*; the supervisor is installed beside them.
        let libexec = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| home.join(".local/libexec/rmac"));
        Ok(Self {
            safe_mode: state_root.join("rmac/session/safe-mode.json"),
            last_safe_mode: state_root.join("rmac/session/safe-mode.last.json"),
            safe_login: runtime_root.join("rmac/safe-mode-login.json"),
            health: runtime_root.join("rmac/session-health.json"),
            libexec,
        })
    }
}
