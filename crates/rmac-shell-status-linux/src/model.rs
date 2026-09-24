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

/// What a `PropertiesChanged` signal means for the shell status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PropertyChange {
    /// Only properties the status never shows: re-reading finds nothing new.
    Unshown,
    /// Only a Wi-Fi access point's signal strength.
    SignalStrength,
    /// Anything else: re-read the service.
    Shown,
}

/// A Wi-Fi signal-strength change re-reads the network at most this often.
/// NetworkManager republishes access-point strength every few seconds, and
/// the bar's signal bars only need to follow it loosely. Not measured on
/// the Mac.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) const SIGNAL_STRENGTH_REFRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// Classify a `PropertiesChanged` signal by its changed property names.
///
/// NetworkManager republishes the Wi-Fi link's bitrate and access-point
/// strength, and UPower the battery's poll time, every few seconds; each used
/// to cost a full re-read of the service on fresh bus connections. The
/// payload is only used to skip or defer a re-read, never as state.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn property_change(
    interface: &str,
    changed: &[&str],
    invalidated: &[&str],
) -> PropertyChange {
    if changed.is_empty() || !invalidated.is_empty() {
        return PropertyChange::Shown;
    }
    let only = |properties: &[&str]| changed.iter().all(|property| properties.contains(property));
    match interface {
        "org.freedesktop.NetworkManager.Device.Wireless" if only(&["Bitrate"]) => {
            PropertyChange::Unshown
        }
        "org.freedesktop.UPower.Device" if only(&["UpdateTime"]) => PropertyChange::Unshown,
        "org.freedesktop.NetworkManager.AccessPoint" if only(&["Strength"]) => {
            PropertyChange::SignalStrength
        }
        _ => PropertyChange::Shown,
    }
}

/// Whether a signal-strength change should re-read the network, given how
/// long ago the network was last re-read (`None`: never).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn signal_strength_refresh_due(since_network_read: Option<std::time::Duration>) -> bool {
    since_network_read.is_none_or(|elapsed| elapsed >= SIGNAL_STRENGTH_REFRESH)
}
