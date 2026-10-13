//! The Win32 reads behind `windows::Facts`: SetupAPI's present devices
//! (only the classes the profile needs), `GetIfTable2`, DXGI's adapters,
//! the display configuration, system metrics and power status. Each is one
//! call or one short enumeration; nothing here waits on a service.

use std::mem::size_of;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, GUID_DEVCLASS_BATTERY,
    GUID_DEVCLASS_BIOMETRIC, GUID_DEVCLASS_BLUETOOTH, GUID_DEVCLASS_CAMERA, GUID_DEVCLASS_HIDCLASS,
    GUID_DEVCLASS_IMAGE, GUID_DEVCLASS_MOUSE, GUID_DEVCLASS_SENSOR, SETUP_DI_REGISTRY_PROPERTY,
    SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME, SPDRP_HARDWAREID, SP_DEVICE_INTERFACE_DATA,
    SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetAutoRotationState, GetDisplayConfigBufferSizes,
    QueryDisplayConfig, AR_NOSENSOR, AR_STATE, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerDeterminePlatformRoleEx, POWER_PLATFORM_ROLE_V2, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GlobalMemoryStatusEx, RelationProcessorCore, MEMORYSTATUSEX,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CONVERTIBLESLATEMODE, SM_DIGITIZER, SM_MAXIMUMTOUCHES,
};

use crate::model::Display;
use crate::windows::{Adapter, Device, Facts, Interface};

const GUID_DEVCLASS_AUDIOENDPOINT: GUID = GUID::from_u128(0xc166523c_fe0c_4a94_a586_f1a80cfbbf3e);
/// ntddvdeo.h: offered by display drivers that can set the panel's
/// brightness.
const GUID_DEVINTERFACE_BRIGHTNESS: GUID = GUID::from_u128(0xfde5bba4_b3f9_46fb_bdaa_0728ce3100b4);
const MAX_DEVICES: u32 = 4096;

fn wide(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

/// A REG_SZ or REG_MULTI_SZ property as strings.
fn property(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &SP_DEVINFO_DATA,
    key: SETUP_DI_REGISTRY_PROPERTY,
) -> Vec<String> {
    let mut buffer = [0u8; 2048];
    let mut size = 0u32;
    // SAFETY: `buffer` outlives the call; SetupAPI writes at most its length.
    let read = unsafe {
        SetupDiGetDeviceRegistryPropertyW(set, data, key, None, Some(&mut buffer), Some(&mut size))
    };
    if read.is_err() {
        return Vec::new();
    }
    let size = (size as usize).min(buffer.len()) / 2;
    let units: Vec<u16> = buffer[..size * 2]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    units
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn class_name(guid: &GUID) -> Option<&'static str> {
    Some(match *guid {
        GUID_DEVCLASS_BATTERY => "Battery",
        GUID_DEVCLASS_BIOMETRIC => "Biometric",
        GUID_DEVCLASS_BLUETOOTH => "Bluetooth",
        GUID_DEVCLASS_CAMERA => "Camera",
        GUID_DEVCLASS_IMAGE => "Image",
        GUID_DEVCLASS_HIDCLASS => "HIDClass",
        GUID_DEVCLASS_MOUSE => "Mouse",
        GUID_DEVCLASS_SENSOR => "Sensor",
        GUID_DEVCLASS_AUDIOENDPOINT => "AudioEndpoint",
        _ => return None,
    })
}

/// Present devices of the classes the profile reads. The class comes with
/// each enumerated device for free; properties are read only for those.
fn devices() -> Vec<Device> {
    let mut found = Vec::new();
    // SAFETY: the set is destroyed below; every structure is sized.
    unsafe {
        let Ok(set) =
            SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
        else {
            return found;
        };
        for index in 0..MAX_DEVICES {
            let mut data = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, index, &mut data).is_err() {
                break;
            }
            let Some(class) = class_name(&data.ClassGuid) else {
                continue;
            };
            let mut id = [0u16; 512];
            let instance_id =
                if SetupDiGetDeviceInstanceIdW(set, &data, Some(&mut id), None).is_ok() {
                    wide(&id)
                } else {
                    String::new()
                };
            let (hardware_ids, name) = if class == "AudioEndpoint" {
                (Vec::new(), String::new())
            } else {
                let name = property(set, &data, SPDRP_FRIENDLYNAME)
                    .into_iter()
                    .next()
                    .or_else(|| property(set, &data, SPDRP_DEVICEDESC).into_iter().next())
                    .unwrap_or_default();
                (property(set, &data, SPDRP_HARDWAREID), name)
            };
            found.push(Device {
                class: class.to_string(),
                instance_id,
                hardware_ids,
                name,
            });
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    found
}

fn brightness_interface() -> bool {
    // SAFETY: as in `devices`.
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_BRIGHTNESS),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        ) else {
            return false;
        };
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        let found = SetupDiEnumDeviceInterfaces(
            set,
            None,
            &GUID_DEVINTERFACE_BRIGHTNESS,
            0,
            &mut interface,
        )
        .is_ok();
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

fn interfaces() -> Vec<Interface> {
    let mut found = Vec::new();
    // SAFETY: the table is freed with FreeMibTable after it is read.
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return found;
        }
        let count = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).Table) as *const MIB_IF_ROW2,
            count,
        );
        for row in rows {
            let flags = row.InterfaceAndOperStatusFlags._bitfield;
            // Bit 0 HardwareInterface, bit 1 FilterInterface (netioapi.h).
            found.push(Interface {
                if_type: row.Type,
                hardware: flags & 1 != 0 && flags & 2 == 0,
                description: wide(&row.Description),
            });
        }
        FreeMibTable(table as *const core::ffi::c_void);
    }
    found
}

fn adapters() -> Vec<Adapter> {
    let mut found = Vec::new();
    // SAFETY: COM-free DXGI factory; interfaces release on drop.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return found;
        };
        for index in 0..16 {
            let Ok(adapter) = factory.EnumAdapters1(index) else {
                break;
            };
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            found.push(Adapter {
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                name: wide(&desc.Description),
                dedicated_memory: desc.DedicatedVideoMemory as u64,
                software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
            });
        }
    }
    found
}

unsafe extern "system" fn monitor(
    handle: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    // SAFETY: `data` is the Vec passed by `monitor_scales`.
    let scales = unsafe { &mut *(data.0 as *mut Vec<(String, f32)>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` is sized as MONITORINFOEXW.
    if unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        let (mut x, mut y) = (96u32, 96u32);
        // SAFETY: plain out-parameters.
        if unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_ok() {
            scales.push((wide(&info.szDevice), x as f32 / 96.0));
        }
    }
    true.into()
}

/// Each monitor's scale by its GDI name (`\\.\DISPLAY1`). Correct only in
/// a per-monitor DPI-aware process (the shell and `rmac-hardware` are).
fn monitor_scales() -> Vec<(String, f32)> {
    let mut scales: Vec<(String, f32)> = Vec::new();
    // SAFETY: the callback runs before this returns.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor),
            LPARAM(&mut scales as *mut Vec<(String, f32)> as isize),
        );
    }
    scales
}

fn displays() -> Vec<Display> {
    let scales = monitor_scales();
    let mut found = Vec::new();
    // SAFETY: the arrays are sized from GetDisplayConfigBufferSizes.
    unsafe {
        let (mut paths, mut modes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes).is_err() {
            return found;
        }
        let mut path_list = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
        let mut mode_list = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut paths,
            path_list.as_mut_ptr(),
            &mut modes,
            mode_list.as_mut_ptr(),
            None,
        )
        .is_err()
        {
            return found;
        }
        path_list.truncate(paths as usize);
        mode_list.truncate(modes as usize);
        for path in &path_list {
            let technology = path.targetInfo.outputTechnology;
            let internal = technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
                || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS;
            let rate = path.targetInfo.refreshRate;
            let refresh = (rate.Denominator > 0)
                .then(|| (u64::from(rate.Numerator) * 1000 / u64::from(rate.Denominator)) as u32);
            let mode_index = path.sourceInfo.Anonymous.modeInfoIdx as usize;
            let (width, height) = mode_list
                .get(mode_index)
                .filter(|mode| mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
                .map(|mode| {
                    let source = mode.Anonymous.sourceMode;
                    (source.width, source.height)
                })
                .unwrap_or((0, 0));
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let name = if DisplayConfigGetDeviceInfo(&mut source.header) == 0 {
                wide(&source.viewGdiDeviceName)
            } else {
                String::new()
            };
            let scale = scales
                .iter()
                .find(|(device, _)| *device == name)
                .map(|(_, scale)| *scale);
            found.push(Display {
                name: name.trim_start_matches("\\\\.\\").to_string(),
                internal,
                width,
                height,
                refresh_millihertz: refresh,
                scale,
                ppi: None,
            });
        }
    }
    found.sort_by_key(|display| (!display.internal, display.name.clone()));
    found
}

fn cpu_cores() -> u32 {
    // SAFETY: the buffer is sized by the first call; records are walked by
    // their own Size.
    unsafe {
        let mut length = 0u32;
        let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut length);
        if length == 0 {
            return 0;
        }
        let mut buffer = vec![0u8; length as usize];
        if GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            Some(buffer.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX),
            &mut length,
        )
        .is_err()
        {
            return 0;
        }
        let mut cores = 0;
        let mut offset = 0usize;
        while offset + 8 <= length as usize {
            let record =
                buffer.as_ptr().add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
            let size = std::ptr::read_unaligned(std::ptr::addr_of!((*record).Size)) as usize;
            if size == 0 {
                break;
            }
            cores += 1;
            offset += size;
        }
        cores
    }
}

pub fn facts() -> Facts {
    let mut power = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter.
    let battery_flag = if unsafe { GetSystemPowerStatus(&mut power) }.is_ok() {
        power.BatteryFlag
    } else {
        255
    };
    let mut memory = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: sized out-parameter.
    let memory_bytes = if unsafe { GlobalMemoryStatusEx(&mut memory) }.is_ok() {
        memory.ullTotalPhys
    } else {
        0
    };
    let mut rotation = AR_STATE::default();
    // SAFETY: plain out-parameter.
    let rotation_sensor =
        unsafe { GetAutoRotationState(&mut rotation) }.as_bool() && rotation.0 & AR_NOSENSOR.0 == 0;
    // SAFETY: no pointers.
    let (digitizer, max_touches, slate, role) = unsafe {
        (
            GetSystemMetrics(SM_DIGITIZER) as u32,
            GetSystemMetrics(SM_MAXIMUMTOUCHES) as u32,
            GetSystemMetrics(SM_CONVERTIBLESLATEMODE) as u32,
            PowerDeterminePlatformRoleEx(POWER_PLATFORM_ROLE_V2).0 as u32,
        )
    };
    let system = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into());
    let cpu_threads = std::thread::available_parallelism()
        .map(|threads| threads.get() as u32)
        .unwrap_or(0);
    let cores = cpu_cores();
    Facts {
        devices: devices(),
        interfaces: interfaces(),
        adapters: adapters(),
        displays: displays(),
        digitizer,
        max_touches,
        // Only convertibles report the slate state meaningfully; laptops
        // without one report 1 (not slate) as well, which reads the same.
        slate_mode: Some(slate),
        platform_role: role,
        battery_flag,
        rotation_sensor,
        brightness_interface: brightness_interface(),
        biometric_service: system.join("System32\\wbiosrvc.dll").exists(),
        memory_bytes,
        cpu_cores: if cores == 0 { cpu_threads } else { cores },
        cpu_threads,
    }
}
