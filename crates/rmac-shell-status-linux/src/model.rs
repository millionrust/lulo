use std::fmt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sources {
    pub network: bool,
    pub bluetooth: bool,
    pub audio: bool,
    pub power: bool,
}

impl Sources {
    pub const fn all() -> Self {
        Self {
            network: true,
            bluetooth: true,
            audio: true,
            power: true,
        }
    }

    pub const fn system_bus() -> Self {
        Self {
            network: true,
            bluetooth: true,
            audio: false,
            power: true,
        }
    }

    pub const fn audio() -> Self {
        Self {
            audio: true,
            ..Self::empty()
        }
    }

    pub const fn empty() -> Self {
        Self {
            network: false,
            bluetooth: false,
            audio: false,
            power: false,
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.network |= other.network;
        self.bluetooth |= other.bluetooth;
        self.audio |= other.audio;
        self.power |= other.power;
    }

    pub const fn is_empty(self) -> bool {
        !self.network && !self.bluetooth && !self.audio && !self.power
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// Re-read the authoritative snapshots for the selected services.
    Refresh(Sources),
    /// A watcher transport is unavailable. Existing snapshots may be stale.
    Unavailable { sources: Sources, detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    #[cfg(target_os = "linux")]
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Whether a `PropertiesChanged` signal changes only properties the shell
/// status never shows, so re-reading the service would find nothing new.
///
/// NetworkManager republishes the Wi-Fi link's bitrate and UPower the
/// battery's poll time every few seconds; each used to cost a full re-read of
/// the service on fresh bus connections. The payload is only used to skip a
/// re-read, never as state.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn only_unshown_properties(
    interface: &str,
    changed: &[&str],
    invalidated: &[&str],
) -> bool {
    let unshown: &[&str] = match interface {
        "org.freedesktop.NetworkManager.Device.Wireless" => &["Bitrate"],
        "org.freedesktop.UPower.Device" => &["UpdateTime"],
        _ => return false,
    };
    !changed.is_empty()
        && invalidated.is_empty()
        && changed.iter().all(|property| unshown.contains(property))
}
