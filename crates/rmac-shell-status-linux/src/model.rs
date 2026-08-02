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
