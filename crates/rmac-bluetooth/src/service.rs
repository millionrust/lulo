use std::fmt;

use crate::watch::system_watch;
use crate::PairingSession;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub address: String,
    pub kind: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub powered: bool,
    pub discoverable: bool,
    pub discovering: bool,
    pub adapter_name: Option<String>,
    pub devices: Vec<Device>,
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
    kind: ErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorKind {
    General,
    Canceled,
    Rejected,
    TimedOut,
}

impl Error {
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
            kind: ErrorKind::General,
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub(crate) fn pairing(
        operation: &'static str,
        detail: impl Into<String>,
        session: &PairingSession,
    ) -> Self {
        let detail = detail.into();
        let outcome = session.outcome();
        let kind = if outcome.timed_out || detail.contains("AuthenticationTimeout") {
            ErrorKind::TimedOut
        } else if outcome.rejected || detail.contains("AuthenticationRejected") {
            ErrorKind::Rejected
        } else if outcome.canceled || detail.contains("AuthenticationCanceled") {
            ErrorKind::Canceled
        } else {
            ErrorKind::General
        };
        let detail = match kind {
            ErrorKind::TimedOut => "pairing confirmation timed out".to_string(),
            ErrorKind::Rejected => "pairing confirmation was rejected".to_string(),
            ErrorKind::Canceled => "pairing was canceled".to_string(),
            ErrorKind::General => detail,
        };
        Self {
            operation,
            detail,
            kind,
        }
    }

    pub fn is_canceled(&self) -> bool {
        self.kind == ErrorKind::Canceled
    }

    pub fn is_rejected(&self) -> bool {
        self.kind == ErrorKind::Rejected
    }

    pub fn is_timed_out(&self) -> bool {
        self.kind == ErrorKind::TimedOut
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub trait BluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_powered(&self, powered: bool) -> Result<(), Error>;
    fn set_discoverable(&self, discoverable: bool) -> Result<(), Error>;
    fn start_discovery(&self) -> Result<(), Error>;
    fn stop_discovery(&self) -> Result<(), Error>;
    fn set_connected(&self, device_id: &str, connected: bool) -> Result<(), Error>;
    fn pair(&self, device_id: &str, session: &PairingSession) -> Result<Snapshot, Error>;
    fn cancel_pairing(&self, device_id: &str) -> Result<(), Error>;
    fn remove_device(&self, device_id: &str) -> Result<Snapshot, Error>;
}

pub struct SystemBluetoothService;

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemBluetoothService.snapshot()
}

pub fn set_powered(powered: bool) -> Result<(), Error> {
    SystemBluetoothService.set_powered(powered)
}

pub fn set_discoverable(discoverable: bool) -> Result<(), Error> {
    SystemBluetoothService.set_discoverable(discoverable)
}

pub fn start_discovery() -> Result<(), Error> {
    SystemBluetoothService.start_discovery()
}

pub fn stop_discovery() -> Result<(), Error> {
    SystemBluetoothService.stop_discovery()
}

pub fn set_connected(device_id: &str, connected: bool) -> Result<(), Error> {
    SystemBluetoothService.set_connected(device_id, connected)
}

pub fn pair(device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
    SystemBluetoothService.pair(device_id, session)
}

pub fn cancel_pairing(device_id: &str) -> Result<(), Error> {
    SystemBluetoothService.cancel_pairing(device_id)
}

pub fn remove_device(device_id: &str) -> Result<Snapshot, Error> {
    SystemBluetoothService.remove_device(device_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    system_watch(sender).await
}
