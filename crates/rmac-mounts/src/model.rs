use std::fmt;
use std::io;
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
pub(crate) const MAX_MOUNTINFO_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_MOUNTS: usize = 256;
pub(crate) const MAX_DISPLAY_NAME_BYTES: usize = 256;
#[cfg(target_os = "linux")]
pub(crate) const MOUNT_WATCH_TIMEOUT_SECONDS: i64 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Opaque identity used only to revalidate a selection against the current
    /// mount namespace. It is not a filesystem path or a display label.
    pub identity: String,
    pub name: String,
    pub path: PathBuf,
    pub ejectable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

impl Usage {
    pub fn used_fraction(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn is_low_space(self) -> bool {
        self.total > 0 && (self.available < 5_000_000_000 || self.available < self.total / 20)
    }

    pub(crate) fn from_blocks(block_size: u64, blocks: u64, available_blocks: u64) -> Self {
        let total = blocks.saturating_mul(block_size);
        let available = available_blocks.saturating_mul(block_size).min(total);
        Self {
            total,
            used: total.saturating_sub(available),
            available,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub mount: Mount,
    pub usage: Option<Usage>,
    pub usage_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[derive(Debug)]
pub enum Error {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Command {
        program: &'static str,
        path: PathBuf,
        message: String,
    },
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    TooManyMounts {
        limit: usize,
    },
    Stale {
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Command {
                program,
                path,
                message,
            } => write!(
                formatter,
                "{program} could not unmount {}: {message}",
                path.display()
            ),
            Self::TooLarge { path, limit } => write!(
                formatter,
                "{} exceeds the {limit}-byte safety limit",
                path.display()
            ),
            Self::TooManyMounts { limit } => {
                write!(formatter, "more than {limit} mounted volumes were reported")
            }
            Self::Stale { name } => {
                write!(
                    formatter,
                    "the mounted volume “{name}” is no longer available"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Command { .. }
            | Self::TooLarge { .. }
            | Self::TooManyMounts { .. }
            | Self::Stale { .. } => None,
        }
    }
}
