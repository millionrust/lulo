//! Stable battery, power-profile, and watch model.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
    #[default]
    Unknown,
}

impl BatteryState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Empty => "Empty",
            Self::FullyCharged => "Fully Charged",
            Self::PendingCharge => "Not Charging",
            Self::PendingDischarge => "Pending Discharge",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    pub percentage: u8,
    pub state: BatteryState,
    pub on_battery: bool,
    pub seconds_remaining: Option<u64>,
    pub capacity: Option<u8>,
    pub charge_cycles: Option<u32>,
    pub energy_rate_watts: Option<f64>,
    pub model: Option<String>,
    pub charge_threshold: ChargeThreshold,
    pub history: BatteryHistory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChargeThresholdAvailability {
    #[default]
    Unsupported,
    MultipleBatteries,
    Available,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChargeThreshold {
    pub availability: ChargeThresholdAvailability,
    pub enabled: bool,
    pub start_percent: Option<u8>,
    pub end_percent: Option<u8>,
    pub firmware_managed: bool,
    pub(super) identity: Option<ChargeThresholdIdentity>,
}

impl Default for ChargeThreshold {
    fn default() -> Self {
        Self {
            availability: ChargeThresholdAvailability::Unsupported,
            enabled: false,
            start_percent: None,
            end_percent: None,
            firmware_managed: false,
            identity: None,
        }
    }
}

impl fmt::Debug for ChargeThreshold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChargeThreshold")
            .field("availability", &self.availability)
            .field("enabled", &self.enabled)
            .field("start_percent", &self.start_percent)
            .field("end_percent", &self.end_percent)
            .field("firmware_managed", &self.firmware_managed)
            .field("has_identity", &self.identity.is_some())
            .finish()
    }
}

impl ChargeThreshold {
    pub fn can_change(&self) -> bool {
        self.availability == ChargeThresholdAvailability::Available && self.identity.is_some()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ChargeThresholdIdentity {
    pub(super) service_owner: String,
    pub(super) object_path: String,
    pub(super) native_path: String,
    pub(super) serial: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryHistoryPoint {
    pub timestamp: u64,
    pub percentage: u8,
    pub state: BatteryState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatteryHistory {
    pub availability: BatteryHistoryAvailability,
    pub points: Vec<BatteryHistoryPoint>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryHistoryAvailability {
    #[default]
    Unsupported,
    Available,
    TemporarilyUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl PowerProfile {
    pub fn id(self) -> &'static str {
        match self {
            Self::PowerSaver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PowerSaver => "Low Power",
            Self::Balanced => "Automatic",
            Self::Performance => "High Power",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub available: bool,
    pub active: Option<PowerProfile>,
    pub supported: Vec<PowerProfile>,
    pub performance_degraded: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub battery: Option<Battery>,
    pub profiles: Profiles,
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
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub fn snapshot() -> Result<Snapshot, Error> {
    system_snapshot()
}

pub fn set_profile(profile: PowerProfile) -> Result<(), Error> {
    system_set_profile(profile)
}

pub fn set_charge_threshold(threshold: &ChargeThreshold, enabled: bool) -> Result<Snapshot, Error> {
    system_set_charge_threshold(threshold, enabled)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    system_watch(sender).await
}

pub(super) fn percent(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}
