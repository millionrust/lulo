//! Cross-platform battery state and system power-profile controls.

use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;

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
    identity: Option<ChargeThresholdIdentity>,
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
struct ChargeThresholdIdentity {
    service_owner: String,
    object_path: String,
    native_path: String,
    serial: String,
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
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
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

#[cfg(any(not(target_os = "macos"), test))]
const UPOWER_SERVICE: &str = "org.freedesktop.UPower";
#[cfg(not(target_os = "macos"))]
const WATCH_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const HISTORY_TIMESPAN_SECONDS: u32 = 24 * 60 * 60;
#[cfg(any(not(target_os = "macos"), test))]
const HISTORY_POINT_LIMIT: usize = 96;
#[cfg(not(target_os = "macos"))]
const THRESHOLD_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(not(target_os = "macos"))]
const THRESHOLD_VERIFY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to the power service", error.to_string()))?;
    system_snapshot_with_connection(&connection)
}

#[cfg(not(target_os = "macos"))]
fn system_snapshot_with_connection(
    connection: &zbus::blocking::Connection,
) -> Result<Snapshot, Error> {
    Ok(Snapshot {
        battery: linux_battery(connection)?,
        profiles: linux_profiles(connection),
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_battery(connection: &zbus::blocking::Connection) -> Result<Option<Battery>, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let upower = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .map_err(|error| Error::new("open UPower", error.to_string()))?;
    let on_battery = upower
        .get_property::<bool>("OnBattery")
        .map_err(|error| Error::new("read the active power source", error.to_string()))?;
    let path = upower
        .call::<_, _, OwnedObjectPath>("GetDisplayDevice", &())
        .map_err(|error| Error::new("find the display battery", error.to_string()))?;
    let device = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        path.as_str(),
        "org.freedesktop.UPower.Device",
    )
    .map_err(|error| Error::new("open the display battery", error.to_string()))?;
    let present = device
        .get_property::<bool>("IsPresent")
        .map_err(|error| Error::new("read battery presence", error.to_string()))?;
    if !present {
        return Ok(None);
    }
    let percentage = device
        .get_property::<f64>("Percentage")
        .map_err(|error| Error::new("read battery charge", error.to_string()))?;
    let state = device
        .get_property::<u32>("State")
        .map(battery_state_from_upower)
        .unwrap_or_default();
    let seconds_remaining = match state {
        BatteryState::Charging | BatteryState::PendingCharge => {
            optional_property::<i64>(&device, "TimeToFull")
        }
        _ => optional_property::<i64>(&device, "TimeToEmpty"),
    }
    .and_then(|seconds| (seconds > 0).then_some(seconds as u64));
    let mut capacity = optional_property::<f64>(&device, "Capacity")
        .filter(|value| *value > 0.0)
        .map(percent);
    let mut charge_cycles = optional_property::<i32>(&device, "ChargeCycles")
        .and_then(|cycles| (cycles >= 0).then_some(cycles as u32));
    let energy_rate_watts =
        optional_property::<f64>(&device, "EnergyRate").filter(|rate| *rate > 0.0);
    let mut model = optional_property::<String>(&device, "Model").filter(|model| !model.is_empty());
    let physical = physical_batteries(connection, &upower).unwrap_or_default();
    if let Some(details) = physical.first() {
        capacity = details.capacity.or(capacity);
        charge_cycles = details.charge_cycles.or(charge_cycles);
        model = details.model.clone().or(model);
    }
    let service_owner = upower_service_owner(connection).ok();
    let charge_threshold = charge_threshold_from_batteries(&physical, service_owner.as_deref());
    let history = battery_history(connection, &device, &physical);
    Ok(Some(Battery {
        percentage: percent(percentage),
        state,
        on_battery,
        seconds_remaining,
        capacity,
        charge_cycles,
        energy_rate_watts,
        model,
        charge_threshold,
        history,
    }))
}

#[cfg(not(target_os = "macos"))]
struct PhysicalBattery {
    object_path: String,
    native_path: String,
    serial: String,
    capacity: Option<u8>,
    charge_cycles: Option<u32>,
    model: Option<String>,
    threshold_supported: bool,
    threshold_enabled: bool,
    threshold_start: Option<u8>,
    threshold_end: Option<u8>,
    threshold_firmware_managed: bool,
}

#[cfg(not(target_os = "macos"))]
fn physical_batteries(
    connection: &zbus::blocking::Connection,
    upower: &zbus::blocking::Proxy<'_>,
) -> Option<Vec<PhysicalBattery>> {
    let paths = upower
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("EnumerateDevices", &())
        .ok()?;
    let mut batteries = Vec::new();
    for path in paths {
        let Ok(device) = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.UPower",
            path.as_str(),
            "org.freedesktop.UPower.Device",
        ) else {
            continue;
        };
        if optional_property::<u32>(&device, "Type") != Some(2)
            || optional_property::<bool>(&device, "PowerSupply") != Some(true)
            || optional_property::<bool>(&device, "IsPresent") != Some(true)
        {
            continue;
        }
        let threshold_settings =
            optional_property::<u32>(&device, "ChargeThresholdSettingsSupported")
                .unwrap_or_default();
        batteries.push(PhysicalBattery {
            object_path: path.to_string(),
            native_path: optional_property::<String>(&device, "NativePath").unwrap_or_default(),
            serial: optional_property::<String>(&device, "Serial").unwrap_or_default(),
            capacity: optional_property::<f64>(&device, "Capacity")
                .filter(|value| *value > 0.0)
                .map(percent),
            charge_cycles: optional_property::<i32>(&device, "ChargeCycles")
                .and_then(|cycles| (cycles >= 0).then_some(cycles as u32)),
            model: optional_property::<String>(&device, "Model").filter(|model| !model.is_empty()),
            threshold_supported: optional_property::<bool>(&device, "ChargeThresholdSupported")
                .unwrap_or(false),
            threshold_enabled: optional_property::<bool>(&device, "ChargeThresholdEnabled")
                .unwrap_or(false),
            threshold_start: threshold_percent(optional_property::<u32>(
                &device,
                "ChargeStartThreshold",
            )),
            threshold_end: threshold_percent(optional_property::<u32>(
                &device,
                "ChargeEndThreshold",
            )),
            threshold_firmware_managed: threshold_settings & 4 != 0,
        });
    }
    Some(batteries)
}

#[cfg(any(not(target_os = "macos"), test))]
fn threshold_percent(value: Option<u32>) -> Option<u8> {
    value.filter(|value| *value <= 100).map(|value| value as u8)
}

#[cfg(not(target_os = "macos"))]
fn charge_threshold_from_batteries(
    batteries: &[PhysicalBattery],
    service_owner: Option<&str>,
) -> ChargeThreshold {
    if batteries.len() > 1 {
        return ChargeThreshold {
            availability: ChargeThresholdAvailability::MultipleBatteries,
            ..ChargeThreshold::default()
        };
    }
    let Some(battery) = batteries.first() else {
        return ChargeThreshold::default();
    };
    let Some(service_owner) = service_owner else {
        return ChargeThreshold::default();
    };
    if !battery.threshold_supported {
        return ChargeThreshold::default();
    }
    ChargeThreshold {
        availability: ChargeThresholdAvailability::Available,
        enabled: battery.threshold_enabled,
        start_percent: battery.threshold_start,
        end_percent: battery.threshold_end,
        firmware_managed: battery.threshold_firmware_managed,
        identity: Some(ChargeThresholdIdentity {
            service_owner: service_owner.to_owned(),
            object_path: battery.object_path.clone(),
            native_path: battery.native_path.clone(),
            serial: battery.serial.clone(),
        }),
    }
}

#[cfg(not(target_os = "macos"))]
fn battery_history(
    connection: &zbus::blocking::Connection,
    display_device: &zbus::blocking::Proxy<'_>,
    physical: &[PhysicalBattery],
) -> BatteryHistory {
    if let Some(history) = history_from_device(display_device) {
        return history;
    }
    if physical.len() != 1 {
        return BatteryHistory::default();
    }
    let Ok(device) = zbus::blocking::Proxy::new(
        connection,
        UPOWER_SERVICE,
        physical[0].object_path.as_str(),
        "org.freedesktop.UPower.Device",
    ) else {
        return BatteryHistory::default();
    };
    history_from_device(&device).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn history_from_device(device: &zbus::blocking::Proxy<'_>) -> Option<BatteryHistory> {
    if optional_property::<bool>(device, "HasHistory") != Some(true) {
        return None;
    }
    let raw = match device.call::<_, _, Vec<(u32, f64, u32)>>(
        "GetHistory",
        &(
            "charge",
            HISTORY_TIMESPAN_SECONDS,
            HISTORY_POINT_LIMIT as u32,
        ),
    ) {
        Ok(raw) => raw,
        Err(_) => {
            return Some(BatteryHistory {
                availability: BatteryHistoryAvailability::TemporarilyUnavailable,
                points: Vec::new(),
            });
        }
    };
    Some(BatteryHistory {
        availability: BatteryHistoryAvailability::Available,
        points: normalize_history(raw),
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn normalize_history(raw: Vec<(u32, f64, u32)>) -> Vec<BatteryHistoryPoint> {
    let mut points = raw
        .into_iter()
        .filter(|(timestamp, value, _)| {
            *timestamp > 0 && value.is_finite() && (0.0..=100.0).contains(value)
        })
        .map(|(timestamp, value, state)| BatteryHistoryPoint {
            timestamp: u64::from(timestamp),
            percentage: percent(value),
            state: battery_state_from_upower(state),
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| point.timestamp);
    points.dedup_by_key(|point| point.timestamp);
    if points.len() > HISTORY_POINT_LIMIT {
        points.drain(..points.len() - HISTORY_POINT_LIMIT);
    }
    points
}

#[cfg(not(target_os = "macos"))]
fn upower_service_owner(connection: &zbus::blocking::Connection) -> Result<String, Error> {
    let dbus = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(|error| Error::new("identify UPower", error.to_string()))?;
    dbus.call::<_, _, String>("GetNameOwner", &(UPOWER_SERVICE,))
        .map_err(|error| Error::new("identify UPower", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn optional_property<T>(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy.get_property(name).ok()
}

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy)]
struct ProfileEndpoint {
    destination: &'static str,
    path: &'static str,
    interface: &'static str,
}

#[cfg(not(target_os = "macos"))]
const PROFILE_ENDPOINTS: [ProfileEndpoint; 2] = [
    ProfileEndpoint {
        destination: "org.freedesktop.UPower.PowerProfiles",
        path: "/org/freedesktop/UPower/PowerProfiles",
        interface: "org.freedesktop.UPower.PowerProfiles",
    },
    ProfileEndpoint {
        destination: "net.hadess.PowerProfiles",
        path: "/net/hadess/PowerProfiles",
        interface: "net.hadess.PowerProfiles",
    },
];

#[cfg(not(target_os = "macos"))]
fn linux_profiles(connection: &zbus::blocking::Connection) -> Profiles {
    PROFILE_ENDPOINTS
        .iter()
        .find_map(|endpoint| profiles_at_endpoint(connection, *endpoint).ok())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn profiles_at_endpoint(
    connection: &zbus::blocking::Connection,
    endpoint: ProfileEndpoint,
) -> Result<Profiles, zbus::Error> {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;

    let proxy = zbus::blocking::Proxy::new(
        connection,
        endpoint.destination,
        endpoint.path,
        endpoint.interface,
    )?;
    let active = proxy
        .get_property::<String>("ActiveProfile")
        .ok()
        .and_then(|profile| parse_profile(&profile));
    let raw = proxy.get_property::<Vec<HashMap<String, OwnedValue>>>("Profiles")?;
    let mut supported = raw
        .iter()
        .filter_map(|profile| property_string(profile, "Profile"))
        .filter_map(|profile| parse_profile(&profile))
        .collect::<Vec<_>>();
    normalize_profiles(&mut supported);
    let performance_degraded = proxy
        .get_property::<String>("PerformanceDegraded")
        .ok()
        .filter(|reason| !reason.is_empty());
    Ok(Profiles {
        available: true,
        active,
        supported,
        performance_degraded,
    })
}

#[cfg(not(target_os = "macos"))]
fn property_string(
    properties: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn system_set_profile(profile: PowerProfile) -> Result<(), Error> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to the power profile service", error.to_string()))?;
    let mut failures = Vec::new();
    for endpoint in PROFILE_ENDPOINTS {
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            endpoint.destination,
            endpoint.path,
            endpoint.interface,
        );
        match proxy {
            Ok(proxy) => match proxy.set_property("ActiveProfile", profile.id()) {
                Ok(()) => return Ok(()),
                Err(error) => failures.push(error.to_string()),
            },
            Err(error) => failures.push(error.to_string()),
        }
    }
    Err(Error::new("change the power profile", failures.join("; ")))
}

#[cfg(not(target_os = "macos"))]
fn system_set_charge_threshold(
    threshold: &ChargeThreshold,
    enabled: bool,
) -> Result<Snapshot, Error> {
    let identity = threshold.identity.as_ref().ok_or_else(|| {
        Error::new(
            "change optimized charging",
            "UPower did not advertise a writable charge threshold for this battery",
        )
    })?;
    if !threshold.can_change() {
        return Err(Error::new(
            "change optimized charging",
            "the captured battery capability is no longer writable",
        ));
    }

    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to UPower", error.to_string()))?;
    let current = revalidate_threshold_battery(&connection, identity)?;
    if current.threshold_enabled != enabled {
        let device = zbus::blocking::Proxy::new(
            &connection,
            identity.service_owner.as_str(),
            identity.object_path.as_str(),
            "org.freedesktop.UPower.Device",
        )
        .map_err(|error| Error::new("open the charge-threshold device", error.to_string()))?;
        device
            .call::<_, _, ()>("EnableChargeThreshold", &(enabled,))
            .map_err(|error| Error::new("change optimized charging", error.to_string()))?;
    }

    let deadline = std::time::Instant::now() + THRESHOLD_VERIFY_TIMEOUT;
    loop {
        let current = revalidate_threshold_battery(&connection, identity)?;
        if current.threshold_enabled == enabled {
            let snapshot = system_snapshot_with_connection(&connection)?;
            let verified = snapshot.battery.as_ref().is_some_and(|battery| {
                battery.charge_threshold.can_change()
                    && battery.charge_threshold.enabled == enabled
                    && battery.charge_threshold.identity.as_ref() == Some(identity)
            });
            if verified {
                return Ok(snapshot);
            }
            return Err(Error::new(
                "verify optimized charging",
                "UPower returned a different battery after the change",
            ));
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify optimized charging",
                "UPower did not confirm the requested state within three seconds",
            ));
        }
        std::thread::sleep(THRESHOLD_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn revalidate_threshold_battery(
    connection: &zbus::blocking::Connection,
    identity: &ChargeThresholdIdentity,
) -> Result<PhysicalBattery, Error> {
    let owner = upower_service_owner(connection)?;
    if owner != identity.service_owner {
        return Err(Error::new(
            "change optimized charging",
            "UPower restarted; refresh Battery before trying again",
        ));
    }
    let upower = zbus::blocking::Proxy::new(
        connection,
        UPOWER_SERVICE,
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .map_err(|error| Error::new("open UPower", error.to_string()))?;
    let mut batteries = physical_batteries(connection, &upower).ok_or_else(|| {
        Error::new(
            "inspect the charge-threshold device",
            "UPower did not return its physical battery inventory",
        )
    })?;
    if upower_service_owner(connection)? != identity.service_owner {
        return Err(Error::new(
            "change optimized charging",
            "UPower restarted during validation; refresh Battery before trying again",
        ));
    }
    if batteries.len() != 1 {
        return Err(Error::new(
            "change optimized charging",
            "the physical battery inventory changed; refresh Battery before trying again",
        ));
    }
    let battery = batteries.remove(0);
    if battery.object_path != identity.object_path
        || battery.native_path != identity.native_path
        || battery.serial != identity.serial
    {
        return Err(Error::new(
            "change optimized charging",
            "the physical battery changed; refresh Battery before trying again",
        ));
    }
    if !battery.threshold_supported {
        return Err(Error::new(
            "change optimized charging",
            "UPower no longer advertises writable charge thresholds for this battery",
        ));
    }
    Ok(battery)
}

#[cfg(not(target_os = "macos"))]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut unavailable_reported = false;
    loop {
        match watch_once(&sender, &mut unavailable_reported).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => publish_unavailable(&sender, &mut unavailable_reported).await?,
        }
        async_io::Timer::after(WATCH_RECONNECT_DELAY).await;
    }
}

#[cfg(target_os = "macos")]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch power changes", "the event consumer closed"))
}

#[cfg(not(target_os = "macos"))]
async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system()
        .await
        .map_err(|error| Error::new("connect power event stream", error.to_string()))?;
    let upower_rule = service_signal_rule(UPOWER_SERVICE, "build UPower signal filter")?;
    let modern_profiles_rule = service_signal_rule(
        PROFILE_ENDPOINTS[0].destination,
        "build power-profile signal filter",
    )?;
    let legacy_profiles_rule = service_signal_rule(
        PROFILE_ENDPOINTS[1].destination,
        "build legacy power-profile signal filter",
    )?;
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("build power owner filter", error.to_string()))?
        .path("/org/freedesktop/DBus")
        .map_err(|error| Error::new("build power owner filter", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("build power owner filter", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("build power owner filter", error.to_string()))?
        .build();
    let mut upower = MessageStream::for_match_rule(upower_rule, &connection, Some(64))
        .await
        .map_err(|error| Error::new("subscribe to UPower changes", error.to_string()))?
        .fuse();
    let mut modern_profiles =
        MessageStream::for_match_rule(modern_profiles_rule, &connection, Some(16))
            .await
            .map_err(|error| Error::new("subscribe to power-profile changes", error.to_string()))?
            .fuse();
    let mut legacy_profiles =
        MessageStream::for_match_rule(legacy_profiles_rule, &connection, Some(16))
            .await
            .map_err(|error| {
                Error::new(
                    "subscribe to legacy power-profile changes",
                    error.to_string(),
                )
            })?
            .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(32))
        .await
        .map_err(|error| Error::new("subscribe to power service restarts", error.to_string()))?
        .fuse();

    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    let service = zbus::names::BusName::try_from(UPOWER_SERVICE)
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    let mut upower_available = dbus
        .name_has_owner(service)
        .await
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    if upower_available {
        publish_changed(sender, unavailable_reported).await?;
    } else {
        publish_unavailable(sender, unavailable_reported).await?;
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = upower.next() => {
                read_signal(message, "read UPower change")?;
                Some(PowerOwnerEvent::Upower(true))
            },
            message = modern_profiles.next() => {
                read_signal(message, "read power-profile change")?;
                Some(PowerOwnerEvent::Profiles)
            },
            message = legacy_profiles.next() => {
                read_signal(message, "read legacy power-profile change")?;
                Some(PowerOwnerEvent::Profiles)
            },
            message = owners.next() => read_owner_event(message)?,
            _ = closed => return Ok(()),
        };
        match event {
            Some(PowerOwnerEvent::Upower(false)) => {
                upower_available = false;
                publish_unavailable(sender, unavailable_reported).await?;
            }
            Some(PowerOwnerEvent::Upower(true)) => {
                upower_available = true;
                publish_changed(sender, unavailable_reported).await?;
            }
            Some(PowerOwnerEvent::Profiles) if upower_available => {
                publish_changed(sender, unavailable_reported).await?;
            }
            _ => {}
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn service_signal_rule(
    service: &'static str,
    operation: &'static str,
) -> Result<zbus::MatchRule<'static>, Error> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(service)
        .map_err(|error| Error::new(operation, error.to_string()))?
        .build();
    Ok(rule)
}

#[cfg(not(target_os = "macos"))]
fn read_signal(
    message: Option<Result<zbus::Message, zbus::Error>>,
    operation: &'static str,
) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(Error::new(operation, error.to_string())),
        None => Err(Error::new(operation, "the signal stream ended")),
    }
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PowerOwnerEvent {
    Upower(bool),
    Profiles,
}

#[cfg(not(target_os = "macos"))]
fn read_owner_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<PowerOwnerEvent>, Error> {
    let message = message
        .ok_or_else(|| Error::new("read power service owner", "the signal stream ended"))?
        .map_err(|error| Error::new("read power service owner", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("read power service owner", error.to_string()))?;
    Ok(power_owner_event(&name, &new_owner))
}

#[cfg(any(not(target_os = "macos"), test))]
fn power_owner_event(name: &str, new_owner: &str) -> Option<PowerOwnerEvent> {
    if name == UPOWER_SERVICE {
        Some(PowerOwnerEvent::Upower(!new_owner.is_empty()))
    } else if name == "org.freedesktop.UPower.PowerProfiles" || name == "net.hadess.PowerProfiles" {
        Some(PowerOwnerEvent::Profiles)
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
async fn publish_changed(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish power recovery", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn publish_unavailable(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish power outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let pmset = command("pmset", &["-g", "batt"], "read battery state")?;
    let ioreg = command(
        "ioreg",
        &["-rn", "AppleSmartBattery"],
        "read battery health",
    )
    .unwrap_or_default();
    Ok(Snapshot {
        battery: parse_macos_battery(&pmset, &ioreg),
        profiles: Profiles::default(),
    })
}

#[cfg(target_os = "macos")]
fn system_set_profile(_: PowerProfile) -> Result<(), Error> {
    Err(Error::new(
        "change the power profile",
        "no supported macOS power-profile adapter is available",
    ))
}

#[cfg(target_os = "macos")]
fn system_set_charge_threshold(_: &ChargeThreshold, _: bool) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change optimized charging",
        "no supported macOS charge-threshold adapter is available",
    ))
}

#[cfg(target_os = "macos")]
fn command(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, Error> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if !output.status.success() {
        return Err(Error::new(
            operation,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_battery(pmset: &str, ioreg: &str) -> Option<Battery> {
    let on_battery = pmset
        .lines()
        .next()
        .is_some_and(|line| line.contains("Battery Power"));
    let line = pmset.lines().find(|line| line.contains('%'))?;
    if line.contains("present: false") {
        return None;
    }
    let parts = line.split(';').collect::<Vec<_>>();
    let percentage = parts
        .first()?
        .split_whitespace()
        .find_map(|field| field.strip_suffix('%'))?
        .parse::<u8>()
        .ok()?
        .min(100);
    let state = match parts.get(1).map(|state| state.trim()) {
        Some("charging") => BatteryState::Charging,
        Some("discharging") => BatteryState::Discharging,
        Some("charged") => BatteryState::FullyCharged,
        Some("finishing charge") => BatteryState::PendingCharge,
        _ => BatteryState::Unknown,
    };
    let seconds_remaining = parts.get(2).and_then(|part| parse_macos_time(part));
    let charge_cycles = ioreg_field(ioreg, "\"CycleCount\"").and_then(|value| value.parse().ok());
    let raw_max =
        ioreg_field(ioreg, "\"AppleRawMaxCapacity\"").and_then(|value| value.parse::<f64>().ok());
    let design =
        ioreg_field(ioreg, "\"DesignCapacity\"").and_then(|value| value.parse::<f64>().ok());
    let capacity = raw_max
        .zip(design)
        .filter(|(_, design)| *design > 0.0)
        .map(|(full, design)| percent(full / design * 100.0));
    Some(Battery {
        percentage,
        state,
        on_battery,
        seconds_remaining,
        capacity,
        charge_cycles,
        energy_rate_watts: None,
        model: None,
        charge_threshold: ChargeThreshold::default(),
        history: BatteryHistory::default(),
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_time(value: &str) -> Option<u64> {
    let time = value.replace("remaining", "");
    let time = time.split_whitespace().next()?;
    let (hours, minutes) = time.split_once(':')?;
    Some(hours.parse::<u64>().ok()? * 3600 + minutes.parse::<u64>().ok()? * 60)
}

#[cfg(any(target_os = "macos", test))]
fn ioreg_field(contents: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = ");
    contents
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&needle))
        .map(str::trim)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

#[cfg(any(not(target_os = "macos"), test))]
fn battery_state_from_upower(state: u32) -> BatteryState {
    match state {
        1 => BatteryState::Charging,
        2 => BatteryState::Discharging,
        3 => BatteryState::Empty,
        4 => BatteryState::FullyCharged,
        5 => BatteryState::PendingCharge,
        6 => BatteryState::PendingDischarge,
        _ => BatteryState::Unknown,
    }
}

fn percent(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_profile(profile: &str) -> Option<PowerProfile> {
    match profile {
        "power-saver" => Some(PowerProfile::PowerSaver),
        "balanced" => Some(PowerProfile::Balanced),
        "performance" => Some(PowerProfile::Performance),
        _ => None,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn normalize_profiles(profiles: &mut Vec<PowerProfile>) {
    profiles.sort_by_key(|profile| match profile {
        PowerProfile::PowerSaver => 0,
        PowerProfile::Balanced => 1,
        PowerProfile::Performance => 2,
    });
    profiles.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upower_states_and_percentages_are_normalized() {
        assert_eq!(battery_state_from_upower(1), BatteryState::Charging);
        assert_eq!(battery_state_from_upower(4), BatteryState::FullyCharged);
        assert_eq!(battery_state_from_upower(99), BatteryState::Unknown);
        assert_eq!(percent(101.0), 100);
        assert_eq!(percent(-1.0), 0);
        assert_eq!(threshold_percent(Some(80)), Some(80));
        assert_eq!(threshold_percent(Some(u32::MAX)), None);
    }

    #[test]
    fn charge_history_is_valid_ordered_unique_and_bounded() {
        let mut raw = (1..=110)
            .rev()
            .map(|timestamp| (timestamp, f64::from(timestamp % 101), 2))
            .collect::<Vec<_>>();
        raw.push((110, 42.0, 1));
        raw.push((0, 50.0, 2));
        raw.push((111, f64::NAN, 2));
        raw.push((112, 101.0, 2));

        let points = normalize_history(raw);
        assert_eq!(points.len(), HISTORY_POINT_LIMIT);
        assert_eq!(points.first().map(|point| point.timestamp), Some(15));
        assert_eq!(points.last().map(|point| point.timestamp), Some(110));
        assert!(points
            .windows(2)
            .all(|points| points[0].timestamp < points[1].timestamp));
    }

    #[test]
    fn threshold_debug_output_redacts_private_identity() {
        let threshold = ChargeThreshold {
            availability: ChargeThresholdAvailability::Available,
            enabled: true,
            start_percent: Some(40),
            end_percent: Some(80),
            firmware_managed: false,
            identity: Some(ChargeThresholdIdentity {
                service_owner: ":1.42".to_owned(),
                object_path: "/org/freedesktop/UPower/devices/battery_BAT0".to_owned(),
                native_path: "/private/device/path".to_owned(),
                serial: "private-serial".to_owned(),
            }),
        };
        let output = format!("{threshold:?}");
        assert!(threshold.can_change());
        assert!(output.contains("has_identity: true"));
        assert!(!output.contains("private"));
        assert!(!output.contains("BAT0"));
    }

    #[test]
    fn profile_order_is_stable_and_duplicates_are_removed() {
        assert_eq!(parse_profile("balanced"), Some(PowerProfile::Balanced));
        assert_eq!(parse_profile("unsupported"), None);
        let mut profiles = vec![
            PowerProfile::Performance,
            PowerProfile::Balanced,
            PowerProfile::PowerSaver,
            PowerProfile::Balanced,
        ];
        normalize_profiles(&mut profiles);
        assert_eq!(
            profiles,
            vec![
                PowerProfile::PowerSaver,
                PowerProfile::Balanced,
                PowerProfile::Performance
            ]
        );
    }

    #[test]
    fn owner_events_distinguish_upower_outages_from_optional_profile_changes() {
        assert_eq!(
            power_owner_event("org.freedesktop.UPower", ""),
            Some(PowerOwnerEvent::Upower(false))
        );
        assert_eq!(
            power_owner_event("org.freedesktop.UPower", ":1.42"),
            Some(PowerOwnerEvent::Upower(true))
        );
        assert_eq!(
            power_owner_event("org.freedesktop.UPower.PowerProfiles", ""),
            Some(PowerOwnerEvent::Profiles)
        );
        assert_eq!(
            power_owner_event("net.hadess.PowerProfiles", ":1.43"),
            Some(PowerOwnerEvent::Profiles)
        );
        assert_eq!(power_owner_event("org.example.Other", ":1.44"), None);
    }

    #[test]
    fn macos_fixture_preserves_charge_health_and_time() {
        let battery = parse_macos_battery(
            "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t63%; discharging; 2:15 remaining present: true",
            "\"CycleCount\" = 242\n\"AppleRawMaxCapacity\" = 4500\n\"DesignCapacity\" = 5000",
        )
        .unwrap();
        assert_eq!(battery.percentage, 63);
        assert_eq!(battery.state, BatteryState::Discharging);
        assert_eq!(battery.seconds_remaining, Some(8100));
        assert_eq!(battery.capacity, Some(90));
        assert_eq!(battery.charge_cycles, Some(242));
        assert!(battery.on_battery);
    }

    #[test]
    fn errors_keep_operation_context() {
        let error = Error::new("read battery state", "service unavailable");
        assert_eq!(
            error.to_string(),
            "could not read battery state: service unavailable"
        );
    }
}
