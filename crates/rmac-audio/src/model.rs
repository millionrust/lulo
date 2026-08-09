//! Stable audio service model.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable,
    Unknown,
}

impl Availability {
    pub fn can_select(self) -> bool {
        self != Self::Unavailable
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Balance {
    pub value: i8,
    pub(super) left_volume: u32,
    pub(super) right_volume: u32,
    pub(super) left_first: bool,
}

impl fmt::Debug for Balance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Balance")
            .field("value", &self.value)
            .field("has_channel_authority", &true)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    pub index: i32,
    pub name: String,
    pub availability: Availability,
    pub is_active: bool,
    pub(super) authority_name: String,
}

impl fmt::Debug for Route {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Route")
            .field("index", &self.index)
            .field("name", &self.name)
            .field("availability", &self.availability)
            .field("is_active", &self.is_active)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Profile {
    pub index: i32,
    pub name: String,
    pub availability: Availability,
    pub is_active: bool,
    pub(super) authority_name: String,
}

impl fmt::Debug for Profile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Profile")
            .field("index", &self.index)
            .field("name", &self.name)
            .field("availability", &self.availability)
            .field("is_active", &self.is_active)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HardwareDevice {
    /// Opaque identifier for this device in the currently sampled audio graph.
    pub id: String,
    pub name: String,
    pub profiles: Vec<Profile>,
    pub(super) authority_name: String,
}

impl fmt::Debug for HardwareDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HardwareDevice")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("profiles", &self.profiles)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    /// Opaque identifier for this node in the currently sampled audio graph.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub routes: Vec<Route>,
    pub balance: Option<Balance>,
    pub(super) authority_name: String,
    pub(super) authority_device_id: Option<String>,
    pub(super) authority_route_device: Option<i32>,
}

impl fmt::Debug for Device {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Device")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("is_default", &self.is_default)
            .field("routes", &self.routes)
            .field("balance", &self.balance)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .field(
                "has_route_authority",
                &(self.authority_device_id.is_some() && self.authority_route_device.is_some()),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultDevice {
    pub name: String,
    pub level: Level,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub has_output: bool,
    pub has_input: bool,
    pub can_set_default: bool,
    pub can_mute_input: bool,
    pub configuration_available: bool,
    pub configuration_error: Option<String>,
    pub output: Level,
    pub input: Level,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub hardware_devices: Vec<HardwareDevice>,
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(super) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}
