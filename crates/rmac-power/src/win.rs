//! Windows power backend: the battery from the power status and the battery
//! class driver, the power profile from the Windows power mode (the power
//! slider's overlay schemes), and changes from power-setting notifications.

use super::*;

use std::sync::OnceLock;

use windows::core::{s, w, GUID, PCSTR, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Power::{
    BatteryDeviceName, BatteryInformation, GetSystemPowerStatus, PowerGetActiveScheme,
    PowerRegisterForEffectivePowerModeNotifications, PowerSettingRegisterNotification,
    PowerSettingUnregisterNotification, PowerUnregisterFromEffectivePowerModeNotifications,
    BATTERY_CAPACITY_RELATIVE, BATTERY_CHARGING, BATTERY_DISCHARGING, BATTERY_INFORMATION,
    BATTERY_POWER_ON_LINE, BATTERY_QUERY_INFORMATION, BATTERY_STATUS, BATTERY_SYSTEM_BATTERY,
    BATTERY_UNKNOWN_CAPACITY, BATTERY_UNKNOWN_RATE, BATTERY_UNKNOWN_TIME, BATTERY_WAIT_STATUS,
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, EFFECTIVE_POWER_MODE, EFFECTIVE_POWER_MODE_V2,
    GUID_DEVICE_BATTERY, HPOWERNOTIFY, IOCTL_BATTERY_QUERY_INFORMATION, IOCTL_BATTERY_QUERY_STATUS,
    IOCTL_BATTERY_QUERY_TAG, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_ACTIVE_POWERSCHEME, GUID_BATTERY_PERCENTAGE_REMAINING,
    GUID_POWERSCHEME_PERSONALITY, GUID_TYPICAL_POWER_SAVINGS,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;

// The power slider's overlay schemes. Windows names them "Best power
// efficiency" / "Better battery", "Balanced" (no overlay), "Better
// performance" and "Best performance".
const OVERLAY_BETTER_BATTERY: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);
const OVERLAY_BETTER_PERFORMANCE: GUID = GUID::from_u128(0x3af9b8d9_7c97_431d_ad78_34a8bfea439f);
const OVERLAY_BEST_PERFORMANCE: GUID = GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238);
const OVERLAY_BALANCED: GUID = GUID::zeroed();

type GetOverlay = unsafe extern "system" fn(*mut GUID) -> u32;
type SetOverlay = unsafe extern "system" fn(GUID) -> u32;

/// powrprof.dll's overlay-scheme functions. They are not in the Windows
/// metadata the `windows` crate is generated from, and older Windows builds
/// lack them, so they are looked up at run time; a missing one means the
/// power mode is unavailable rather than a launch failure. Each returns a
/// Win32 error code.
#[derive(Default)]
struct OverlayApi {
    get_actual: Option<GetOverlay>,
    get_effective: Option<GetOverlay>,
    set_active: Option<SetOverlay>,
}

fn overlay_api() -> &'static OverlayApi {
    static API: OnceLock<OverlayApi> = OnceLock::new();
    API.get_or_init(|| {
        // SAFETY: the module stays loaded for the life of the process, and
        // each export has the signature declared by its type.
        unsafe {
            let Ok(module) = LoadLibraryExW(w!("powrprof.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            else {
                return OverlayApi::default();
            };
            let get = |name: PCSTR| {
                GetProcAddress(module, name).map(|function| {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, GetOverlay>(
                        function,
                    )
                })
            };
            OverlayApi {
                get_actual: get(s!("PowerGetActualOverlayScheme")),
                get_effective: get(s!("PowerGetEffectiveOverlayScheme")),
                set_active: GetProcAddress(module, s!("PowerSetActiveOverlayScheme")).map(
                    |function| {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, SetOverlay>(
                            function,
                        )
                    },
                ),
            }
        }
    })
}

pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    if let Some(battery) = fake_battery() {
        return Ok(Snapshot {
            battery: Some(battery),
            profiles: read_profiles(),
        });
    }
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter.
    unsafe { GetSystemPowerStatus(&mut status) }
        .map_err(|error| Error::new("read battery state", error.message()))?;
    let battery = if status.BatteryFlag & 128 != 0 {
        None
    } else {
        read_battery(&status, battery_details())
    };
    Ok(Snapshot {
        battery,
        profiles: read_profiles(),
    })
}

pub(super) fn system_set_profile(profile: PowerProfile) -> Result<(), Error> {
    if !balanced_scheme_active() {
        return Err(Error::new(
            "change the power profile",
            "Windows power modes need the Balanced power plan",
        ));
    }
    let Some(set_active) = overlay_api().set_active else {
        return Err(Error::new(
            "change the power profile",
            "this Windows version has no power modes",
        ));
    };
    // SAFETY: the GUID is passed by value.
    let result = unsafe { set_active(overlay_for_profile(profile)) };
    if result != ERROR_SUCCESS.0 {
        return Err(Error::new(
            "change the power profile",
            windows::core::HRESULT::from_win32(result).message(),
        ));
    }
    Ok(())
}

pub(super) fn system_set_charge_threshold(_: &ChargeThreshold, _: bool) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change optimized charging",
        "Windows has no charge-limit interface; the PC maker's own tool controls it",
    ))
}

pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let registrations = Registrations::register(sender.clone());
    if registrations.is_empty() {
        return sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("watch power changes", "the event consumer closed"));
    }
    sender.closed().await;
    drop(registrations);
    Ok(())
}

/// What the battery class driver reports for the first system battery.
#[derive(Clone, Debug, Default, PartialEq)]
struct BatteryDetails {
    designed_capacity: Option<u32>,
    full_charged_capacity: Option<u32>,
    remaining_capacity: Option<u32>,
    /// Capacities are in mWh and the rate in mW (not relative units).
    absolute_units: bool,
    cycle_count: Option<u32>,
    /// Charge or discharge rate in mW; negative while discharging.
    rate: Option<i32>,
    charging: bool,
    discharging: bool,
    on_line: bool,
    model: Option<String>,
}

fn read_battery(status: &SYSTEM_POWER_STATUS, details: Option<BatteryDetails>) -> Option<Battery> {
    // 255: the status is unknown; trust it only if the driver found a battery.
    if status.BatteryFlag == 255 && details.is_none() {
        return None;
    }
    let details = details.unwrap_or_default();
    let percentage = if status.BatteryLifePercent <= 100 {
        status.BatteryLifePercent
    } else {
        let full = details.full_charged_capacity.filter(|full| *full > 0)?;
        let remaining = details.remaining_capacity?;
        percent(f64::from(remaining) / f64::from(full) * 100.0)
    };
    let on_battery = status.ACLineStatus == 0;
    let state = if details.charging || details.discharging || details.on_line {
        battery_state(
            details.charging,
            details.discharging,
            details.on_line,
            percentage,
        )
    } else {
        // 8: charging.
        battery_state(
            status.BatteryFlag & 8 != 0 && status.BatteryFlag != 255,
            on_battery,
            status.ACLineStatus == 1,
            percentage,
        )
    };
    let seconds_remaining = match state {
        BatteryState::Discharging => (status.BatteryLifeTime != BATTERY_UNKNOWN_TIME
            && status.BatteryLifeTime > 0)
            .then_some(u64::from(status.BatteryLifeTime)),
        BatteryState::Charging if details.absolute_units => time_to_full(
            details.full_charged_capacity,
            details.remaining_capacity,
            details.rate,
        ),
        _ => None,
    };
    let energy_rate_watts = details
        .rate
        .filter(|_| details.absolute_units)
        .filter(|rate| *rate != 0)
        .map(|rate| f64::from(rate.unsigned_abs()) / 1000.0);
    Some(Battery {
        percentage,
        state,
        on_battery,
        seconds_remaining,
        capacity: health(details.designed_capacity, details.full_charged_capacity),
        charge_cycles: details.cycle_count,
        energy_rate_watts,
        model: details.model,
        charge_threshold: ChargeThreshold::default(),
        history: BatteryHistory::default(),
    })
}

/// A stand-in battery for the Windows shell checks on CI runners, which
/// have none (`scripts/windows/shell_smoke.py`):
/// `LULO_FAKE_BATTERY=<percent>,<charging|charged|not-charging|discharging>[,<seconds>]`,
/// the seconds being the time to full or to empty.
fn fake_battery() -> Option<Battery> {
    parse_fake_battery(&std::env::var("LULO_FAKE_BATTERY").ok()?)
}

fn parse_fake_battery(value: &str) -> Option<Battery> {
    let mut parts = value.split(',').map(str::trim);
    let percentage = parts.next()?.parse::<u8>().ok()?.min(100);
    let state = match parts.next()? {
        "charging" => BatteryState::Charging,
        "charged" => BatteryState::FullyCharged,
        "not-charging" => BatteryState::PendingCharge,
        "discharging" => BatteryState::Discharging,
        _ => return None,
    };
    let seconds_remaining = parts.next().and_then(|seconds| seconds.parse().ok());
    Some(Battery {
        percentage,
        state,
        on_battery: state == BatteryState::Discharging,
        seconds_remaining,
        capacity: None,
        charge_cycles: None,
        energy_rate_watts: None,
        model: None,
        charge_threshold: ChargeThreshold::default(),
        history: BatteryHistory::default(),
    })
}

fn battery_state(charging: bool, discharging: bool, on_line: bool, percentage: u8) -> BatteryState {
    if charging {
        BatteryState::Charging
    } else if discharging {
        BatteryState::Discharging
    } else if on_line && percentage >= 100 {
        BatteryState::FullyCharged
    } else if on_line {
        BatteryState::PendingCharge
    } else {
        BatteryState::Unknown
    }
}

/// Seconds until full from mWh capacities and a mW charge rate.
fn time_to_full(full: Option<u32>, remaining: Option<u32>, rate: Option<i32>) -> Option<u64> {
    let full = u64::from(full?);
    let remaining = u64::from(remaining?);
    let rate = u64::try_from(rate?).ok().filter(|rate| *rate > 0)?;
    let missing = full.checked_sub(remaining).filter(|missing| *missing > 0)?;
    Some(missing * 3600 / rate)
}

/// Full-charge capacity as a percentage of the design capacity.
fn health(designed: Option<u32>, full: Option<u32>) -> Option<u8> {
    let designed = designed.filter(|designed| *designed > 0)?;
    let full = full.filter(|full| *full > 0)?;
    Some(percent(f64::from(full) / f64::from(designed) * 100.0))
}

fn known_capacity(value: u32) -> Option<u32> {
    (value != BATTERY_UNKNOWN_CAPACITY).then_some(value)
}

/// The first system battery the battery class driver lists, if any.
fn battery_details() -> Option<BatteryDetails> {
    // SAFETY: the device list is destroyed after use; each interface detail
    // buffer is sized from the size Windows reports and aligned for the struct.
    unsafe {
        let devices = SetupDiGetClassDevsW(
            Some(&GUID_DEVICE_BATTERY as *const GUID),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .ok()?;
        let mut found = None;
        let mut index = 0;
        loop {
            let mut interface = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(
                devices,
                None,
                &GUID_DEVICE_BATTERY,
                index,
                &mut interface,
            )
            .is_err()
            {
                break;
            }
            index += 1;
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                devices,
                &interface,
                None,
                0,
                Some(&mut required as *mut u32),
                None,
            );
            if (required as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
                continue;
            }
            let mut buffer = vec![0u64; (required as usize).div_ceil(size_of::<u64>())];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                devices,
                &interface,
                Some(detail),
                required,
                None,
                None,
            )
            .is_err()
            {
                continue;
            }
            let path = PCWSTR((*detail).DevicePath.as_ptr());
            if let Some(details) = query_battery(path) {
                found = Some(details);
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(devices);
        found
    }
}

/// Reads one battery device; `None` if it is not a system battery or has
/// no battery in its slot.
///
/// # Safety
/// `path` must point to a NUL-terminated device path.
unsafe fn query_battery(path: PCWSTR) -> Option<BatteryDetails> {
    let handle = unsafe {
        CreateFileW(
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;
    let details = unsafe { query_open_battery(handle) };
    // SAFETY: the handle was opened above and is not used again.
    let _ = unsafe { CloseHandle(handle) };
    details
}

/// # Safety
/// `handle` must be an open battery device handle.
unsafe fn query_open_battery(handle: HANDLE) -> Option<BatteryDetails> {
    // A zero timeout returns the current tag at once; tag 0 is "no battery".
    let wait = 0u32;
    let mut tag = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_TAG,
            Some(&wait as *const u32 as *const core::ffi::c_void),
            size_of::<u32>() as u32,
            Some(&mut tag as *mut u32 as *mut core::ffi::c_void),
            size_of::<u32>() as u32,
            None,
            None,
        )
    }
    .ok()?;
    if tag == 0 {
        return None;
    }
    let mut query = BATTERY_QUERY_INFORMATION {
        BatteryTag: tag,
        InformationLevel: BatteryInformation,
        AtRate: 0,
    };
    let mut information = BATTERY_INFORMATION::default();
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_INFORMATION,
            Some(&query as *const BATTERY_QUERY_INFORMATION as *const core::ffi::c_void),
            size_of::<BATTERY_QUERY_INFORMATION>() as u32,
            Some(&mut information as *mut BATTERY_INFORMATION as *mut core::ffi::c_void),
            size_of::<BATTERY_INFORMATION>() as u32,
            None,
            None,
        )
    }
    .ok()?;
    // UPS units also appear in the battery class; only system batteries count.
    if information.Capabilities & BATTERY_SYSTEM_BATTERY == 0 {
        return None;
    }
    let wait_status = BATTERY_WAIT_STATUS {
        BatteryTag: tag,
        ..Default::default()
    };
    let mut status = BATTERY_STATUS::default();
    let status_read = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_STATUS,
            Some(&wait_status as *const BATTERY_WAIT_STATUS as *const core::ffi::c_void),
            size_of::<BATTERY_WAIT_STATUS>() as u32,
            Some(&mut status as *mut BATTERY_STATUS as *mut core::ffi::c_void),
            size_of::<BATTERY_STATUS>() as u32,
            None,
            None,
        )
    }
    .is_ok();
    query.InformationLevel = BatteryDeviceName;
    let mut name = [0u16; 128];
    let mut name_bytes = 0u32;
    let model = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_INFORMATION,
            Some(&query as *const BATTERY_QUERY_INFORMATION as *const core::ffi::c_void),
            size_of::<BATTERY_QUERY_INFORMATION>() as u32,
            Some(name.as_mut_ptr() as *mut core::ffi::c_void),
            size_of_val(&name) as u32,
            Some(&mut name_bytes as *mut u32),
            None,
        )
    }
    .ok()
    .and_then(|()| wide_string(&name, name_bytes as usize / 2));
    let rate = (status_read && status.Rate as u32 != BATTERY_UNKNOWN_RATE).then_some(status.Rate);
    Some(BatteryDetails {
        designed_capacity: known_capacity(information.DesignedCapacity),
        full_charged_capacity: known_capacity(information.FullChargedCapacity),
        remaining_capacity: status_read
            .then_some(status.Capacity)
            .and_then(known_capacity),
        absolute_units: information.Capabilities & BATTERY_CAPACITY_RELATIVE == 0,
        // Drivers that do not count cycles report 0.
        cycle_count: (information.CycleCount > 0).then_some(information.CycleCount),
        rate,
        charging: status_read && status.PowerState & BATTERY_CHARGING != 0,
        discharging: status_read && status.PowerState & BATTERY_DISCHARGING != 0,
        on_line: status_read && status.PowerState & BATTERY_POWER_ON_LINE != 0,
        model,
    })
}

fn wide_string(buffer: &[u16], length: usize) -> Option<String> {
    let text = &buffer[..length.min(buffer.len())];
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    let name = String::from_utf16_lossy(&text[..end]).trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn read_profiles() -> Profiles {
    if !balanced_scheme_active() {
        return Profiles::default();
    }
    let api = overlay_api();
    // The actual overlay is the user's choice; the effective one can differ
    // while Energy Saver is on, so it only stands in when the first is missing.
    let Some(overlay) = [api.get_actual, api.get_effective]
        .into_iter()
        .flatten()
        .find_map(|get| {
            let mut overlay = GUID::zeroed();
            // SAFETY: plain out-parameter.
            (unsafe { get(&mut overlay) } == ERROR_SUCCESS.0).then_some(overlay)
        })
    else {
        return Profiles::default();
    };
    Profiles {
        available: true,
        active: profile_for_overlay(overlay),
        supported: vec![
            PowerProfile::PowerSaver,
            PowerProfile::Balanced,
            PowerProfile::Performance,
        ],
        performance_degraded: None,
    }
}

/// Windows offers power modes only on top of the Balanced power plan; a
/// custom or High performance plan turns the slider off.
fn balanced_scheme_active() -> bool {
    let mut scheme: *mut GUID = std::ptr::null_mut();
    // SAFETY: Windows allocates the GUID; it is copied, then freed with LocalFree.
    unsafe {
        if PowerGetActiveScheme(None, &mut scheme) != ERROR_SUCCESS || scheme.is_null() {
            return false;
        }
        let active = *scheme;
        let _ = LocalFree(Some(HLOCAL(scheme as *mut core::ffi::c_void)));
        active == GUID_TYPICAL_POWER_SAVINGS
    }
}

fn profile_for_overlay(overlay: GUID) -> Option<PowerProfile> {
    match overlay {
        OVERLAY_BETTER_BATTERY => Some(PowerProfile::PowerSaver),
        OVERLAY_BALANCED | OVERLAY_BETTER_PERFORMANCE => Some(PowerProfile::Balanced),
        OVERLAY_BEST_PERFORMANCE => Some(PowerProfile::Performance),
        _ => None,
    }
}

fn overlay_for_profile(profile: PowerProfile) -> GUID {
    match profile {
        PowerProfile::PowerSaver => OVERLAY_BETTER_BATTERY,
        PowerProfile::Balanced => OVERLAY_BALANCED,
        PowerProfile::Performance => OVERLAY_BEST_PERFORMANCE,
    }
}

/// The power-setting and power-mode registrations of one watch. Dropping it
/// unregisters them before the callback context is freed.
struct Registrations {
    settings: Vec<HPOWERNOTIFY>,
    power_mode: Option<*mut core::ffi::c_void>,
    // Windows holds pointers to both for as long as the registrations live.
    parameters: *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
    sender: *mut async_channel::Sender<WatchEvent>,
}

// SAFETY: the registration handles and the boxed context are only touched
// on drop; the callbacks only call `try_send` on the `Sender`, which is
// `Send + Sync`.
unsafe impl Send for Registrations {}

impl Registrations {
    fn register(sender: async_channel::Sender<WatchEvent>) -> Self {
        let sender = Box::into_raw(Box::new(sender));
        let parameters = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power_setting),
            Context: sender as *mut core::ffi::c_void,
        }));
        let mut settings = Vec::new();
        for setting in [
            GUID_ACDC_POWER_SOURCE,
            GUID_BATTERY_PERCENTAGE_REMAINING,
            GUID_POWERSCHEME_PERSONALITY,
            GUID_ACTIVE_POWERSCHEME,
        ] {
            let mut registration: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: the parameters and their context outlive the
            // registration (see Drop).
            let result = unsafe {
                PowerSettingRegisterNotification(
                    &setting,
                    DEVICE_NOTIFY_CALLBACK,
                    HANDLE(parameters as *mut core::ffi::c_void),
                    &mut registration,
                )
            };
            if result == ERROR_SUCCESS && !registration.is_null() {
                settings.push(HPOWERNOTIFY(registration as isize));
            }
        }
        // The effective power mode also changes when the power slider moves.
        let mut power_mode: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: the context outlives the registration (see Drop).
        let registered = unsafe {
            PowerRegisterForEffectivePowerModeNotifications(
                EFFECTIVE_POWER_MODE_V2,
                Some(on_power_mode),
                Some(sender as *const core::ffi::c_void),
                &mut power_mode,
            )
        }
        .is_ok();
        Self {
            settings,
            power_mode: (registered && !power_mode.is_null()).then_some(power_mode),
            parameters,
            sender,
        }
    }

    fn is_empty(&self) -> bool {
        self.settings.is_empty() && self.power_mode.is_none()
    }
}

impl Drop for Registrations {
    fn drop(&mut self) {
        // SAFETY: each handle came from a successful registration; once all
        // are unregistered no callback can use the context, so it is freed.
        unsafe {
            for registration in self.settings.drain(..) {
                let _ = PowerSettingUnregisterNotification(registration);
            }
            if let Some(registration) = self.power_mode.take() {
                let _ = PowerUnregisterFromEffectivePowerModeNotifications(registration);
            }
            drop(Box::from_raw(self.parameters));
            drop(Box::from_raw(self.sender));
        }
    }
}

/// # Safety
/// `context` is the `Sender` a live [`Registrations`] owns.
unsafe fn notify(context: *const core::ffi::c_void) {
    if let Some(sender) = unsafe { (context as *const async_channel::Sender<WatchEvent>).as_ref() }
    {
        // A full channel already holds a pending change.
        let _ = sender.try_send(WatchEvent::Changed);
    }
}

unsafe extern "system" fn on_power_setting(
    context: *const core::ffi::c_void,
    _kind: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    unsafe { notify(context) };
    0
}

unsafe extern "system" fn on_power_mode(
    _mode: EFFECTIVE_POWER_MODE,
    context: *const core::ffi::c_void,
) {
    unsafe { notify(context) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlays_map_to_profiles_and_back() {
        for profile in [
            PowerProfile::PowerSaver,
            PowerProfile::Balanced,
            PowerProfile::Performance,
        ] {
            assert_eq!(
                profile_for_overlay(overlay_for_profile(profile)),
                Some(profile)
            );
        }
        assert_eq!(
            profile_for_overlay(OVERLAY_BETTER_PERFORMANCE),
            Some(PowerProfile::Balanced)
        );
        assert_eq!(profile_for_overlay(GUID::from_u128(1)), None);
    }

    #[test]
    fn the_checks_stand_in_battery_reads_its_state() {
        let battery = parse_fake_battery("85, charging, 3900").unwrap();
        assert_eq!(
            (battery.percentage, battery.state, battery.on_battery),
            (85, BatteryState::Charging, false)
        );
        assert_eq!(battery.seconds_remaining, Some(3900));
        let battery = parse_fake_battery("40,discharging").unwrap();
        assert!(battery.on_battery && battery.seconds_remaining.is_none());
        assert!(parse_fake_battery("40,sideways").is_none());
    }

    #[test]
    fn battery_states_follow_the_driver_flags() {
        assert_eq!(battery_state(true, false, true, 50), BatteryState::Charging);
        assert_eq!(
            battery_state(false, true, false, 50),
            BatteryState::Discharging
        );
        assert_eq!(
            battery_state(false, false, true, 100),
            BatteryState::FullyCharged
        );
        assert_eq!(
            battery_state(false, false, true, 80),
            BatteryState::PendingCharge
        );
        assert_eq!(
            battery_state(false, false, false, 80),
            BatteryState::Unknown
        );
    }

    #[test]
    fn health_and_time_to_full_are_computed_from_capacities() {
        assert_eq!(health(Some(50_000), Some(45_000)), Some(90));
        assert_eq!(health(Some(0), Some(45_000)), None);
        assert_eq!(health(Some(50_000), None), None);
        assert_eq!(
            time_to_full(Some(50_000), Some(40_000), Some(20_000)),
            Some(1800)
        );
        assert_eq!(time_to_full(Some(50_000), Some(50_000), Some(20_000)), None);
        assert_eq!(time_to_full(Some(50_000), Some(40_000), Some(-5_000)), None);
        assert_eq!(known_capacity(BATTERY_UNKNOWN_CAPACITY), None);
    }

    #[test]
    fn device_names_stop_at_the_terminator() {
        let mut buffer = [0u16; 8];
        for (slot, unit) in buffer.iter_mut().zip("Bat1".encode_utf16()) {
            *slot = unit;
        }
        assert_eq!(wide_string(&buffer, 16), Some("Bat1".to_string()));
        assert_eq!(wide_string(&[0u16; 4], 8), None);
    }
}
