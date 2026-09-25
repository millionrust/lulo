//! The Automatic Updates switches and the menu bar's update count.
//!
//! Both are tiny `key=value` files shared with `rmac-update-check` (Python),
//! which reads the switches on every timer run and writes the count after
//! every check. System Settings writes the switches and, after each
//! snapshot, the count. The menu bar reads the count when the Apple menu
//! opens; nothing watches or polls either file.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::*;

const MAX_FILE_BYTES: u64 = 4096;
const SETTINGS_HEADER: &str =
    "# Lulo OS Software Update (System Settings > General > Software Update)";

/// macOS 26.2's three Automatic Updates switches. The Mac always checks;
/// so does the daily timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomaticUpdates {
    /// "Download new updates when available".
    pub download: bool,
    /// "Install Lulo OS updates" (the Mac's "Install macOS updates").
    pub install_lulo_os: bool,
    /// "Install system data files and security updates".
    pub install_security: bool,
}

impl Default for AutomaticUpdates {
    fn default() -> Self {
        Self {
            download: true,
            install_lulo_os: true,
            install_security: true,
        }
    }
}

impl AutomaticUpdates {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = match value.trim() {
                "true" => true,
                "false" => false,
                _ => continue,
            };
            match key.trim() {
                "download-updates" => settings.download = value,
                "install-lulo-os" => settings.install_lulo_os = value,
                "install-security" => settings.install_security = value,
                _ => {}
            }
        }
        settings
    }

    pub fn render(&self) -> String {
        format!(
            "{SETTINGS_HEADER}\ndownload-updates={}\ninstall-lulo-os={}\ninstall-security={}\n",
            self.download, self.install_lulo_os, self.install_security
        )
    }

    /// The pane's "Automatic Updates  On/Off" value. The install switches
    /// only act on downloaded updates, so downloading decides it.
    pub fn summary(&self) -> &'static str {
        if self.download {
            "On"
        } else {
            "Off"
        }
    }

    pub fn load(path: &Path) -> Self {
        read_bounded(path).map_or_else(Self::default, |text| Self::parse(&text))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, &self.render())
    }

    pub fn default_path() -> Option<PathBuf> {
        xdg_dir("XDG_CONFIG_HOME", ".config").map(|dir| dir.join("rmac/software-update.conf"))
    }
}

/// What the menu bar shows next to "System Settings…".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpdateStatus {
    pub updates: usize,
    pub restart_required: bool,
}

impl UpdateStatus {
    pub fn parse(text: &str) -> Self {
        let mut status = Self::default();
        let mut version_ok = false;
        for line in text.lines() {
            match line.trim().split_once('=') {
                Some(("version", "1")) => version_ok = true,
                Some(("updates", value)) => {
                    status.updates = value.parse::<usize>().unwrap_or(0).min(MAX_UPDATES)
                }
                Some(("restart-required", value)) => status.restart_required = value == "1",
                _ => {}
            }
        }
        if version_ok {
            status
        } else {
            Self::default()
        }
    }

    pub fn render(&self) -> String {
        format!(
            "version=1\nupdates={}\nrestart-required={}\n",
            self.updates,
            u8::from(self.restart_required)
        )
    }

    /// "1 update" / "3 updates", or `None` for nothing to show.
    pub fn badge(&self) -> Option<String> {
        match self.updates {
            0 => None,
            1 => Some("1 update".into()),
            count => Some(format!("{count} updates")),
        }
    }

    pub fn load(path: &Path) -> Self {
        read_bounded(path).map_or_else(Self::default, |text| Self::parse(&text))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, &self.render())
    }

    pub fn default_path() -> Option<PathBuf> {
        xdg_dir("XDG_STATE_HOME", ".local/state").map(|dir| dir.join("rmac/software-update-status"))
    }
}

fn xdg_dir(variable: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(fallback))
        })
}

fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read as _;

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// Write through a private temporary file in the same directory and rename
/// it into place, so a reader never sees half a file.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent"))?;
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    // A crash of an earlier process with the same PID can leave one behind.
    let _ = std::fs::remove_file(&temporary);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
