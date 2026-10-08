//! The one place the Windows Settings reaches Windows itself (ADR 0023
//! phase 2e): the facts About This PC and Displays show, the output volume
//! Sound reads out, and setting the Windows desktop wallpaper.
//!
//! The panes see only the [`Host`] trait. [`WindowsHost`] answers it with
//! Win32 calls; tests answer it with fixed facts. Lulo's own preferences
//! (theme, wallpaper, alert sound) are not here: they live in the
//! platform-neutral stores every Lulo app already reads (`rmac-theme`,
//! `rmac-shell-settings`, `rmac-sound`).
//!
//! Every call reads the system once and returns; none of them waits on
//! anything, polls or keeps a thread. The panes call them off the UI
//! thread.

use std::path::Path;
use std::sync::Arc;

use rmac_shell_settings::WallpaperFit;

/// What About This PC shows. A fact Windows did not report is `None` (or
/// empty) and its row is left out rather than guessed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AboutFacts {
    pub(crate) computer_name: Option<String>,
    /// The maker and model the firmware reports, such as "HP Laptop 15s".
    pub(crate) model: Option<String>,
    pub(crate) processor: Option<String>,
    /// Physical cores and logical processors.
    pub(crate) cores: Option<(usize, usize)>,
    /// Installed memory in bytes.
    pub(crate) memory: Option<u64>,
    pub(crate) graphics: Vec<String>,
    pub(crate) os: Option<OsVersion>,
    pub(crate) drives: Vec<Drive>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OsVersion {
    /// `ProductName`, such as "Windows 10 Home" (Windows 11 still says 10).
    pub(crate) product: String,
    /// `DisplayVersion`, such as "24H2".
    pub(crate) release: Option<String>,
    pub(crate) build: Option<u32>,
    pub(crate) revision: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Drive {
    pub(crate) name: String,
    pub(crate) total: u64,
    pub(crate) available: u64,
}

/// One connected display, as Windows drives it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplayFacts {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) refresh_hz: Option<u32>,
    /// Windows' "Scale" setting for the display, in percent.
    pub(crate) scale_percent: u32,
    pub(crate) primary: bool,
    pub(crate) built_in: bool,
}

/// The default output device and its master volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VolumeReading {
    pub(crate) device: Option<String>,
    pub(crate) percent: u8,
    pub(crate) muted: bool,
}

/// The Windows half of the Settings panes.
pub(crate) trait Host: Send + Sync {
    fn about(&self) -> AboutFacts;
    fn displays(&self) -> Result<Vec<DisplayFacts>, String>;
    fn output_volume(&self) -> Result<VolumeReading, String>;
    /// Make `image` (a JPEG, PNG or BMP file) the Windows desktop picture,
    /// placed the way `fit` says, for every display.
    fn set_desktop_wallpaper(&self, image: &Path, fit: WallpaperFit) -> Result<(), String>;
}

/// The host the app runs against.
pub(crate) fn current() -> Arc<dyn Host> {
    Arc::new(WindowsHost)
}

// ---- formatting the facts --------------------------------------------------

/// Installed memory as the Mac's About shows it: whole gigabytes ("8 GB"),
/// in the binary units memory is sold in.
pub(crate) fn memory_label(bytes: u64) -> String {
    let gib = bytes as f64 / (1u64 << 30) as f64;
    if gib >= 1.0 {
        format!("{} GB", gib.round() as u64)
    } else {
        format!("{} MB", (bytes as f64 / (1u64 << 20) as f64).round() as u64)
    }
}

/// A capacity in decimal units, as storage is labelled: "237.32 GB".
pub(crate) fn capacity_label(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// "Windows 11 Home 24H2 (build 26200.6584)". Windows 11 keeps
/// `ProductName` at "Windows 10 …"; its builds start at 22000.
pub(crate) fn os_label(os: &OsVersion) -> String {
    let mut product = os.product.trim().to_owned();
    if os.build.is_some_and(|build| build >= 22_000) && product.starts_with("Windows 10") {
        product = product.replacen("Windows 10", "Windows 11", 1);
    }
    let mut label = product;
    if let Some(release) = os.release.as_deref().filter(|release| !release.is_empty()) {
        label.push(' ');
        label.push_str(release);
    }
    match (os.build, os.revision) {
        (Some(build), Some(revision)) => label.push_str(&format!(" (build {build}.{revision})")),
        (Some(build), None) => label.push_str(&format!(" (build {build})")),
        _ => {}
    }
    label
}

/// "4 cores, 8 threads"; one figure when they are equal.
pub(crate) fn cores_label((cores, threads): (usize, usize)) -> String {
    let plural = |count: usize, word: &str| {
        if count == 1 {
            format!("1 {word}")
        } else {
            format!("{count} {word}s")
        }
    };
    if cores == threads || threads == 0 {
        plural(cores, "core")
    } else {
        format!("{}, {}", plural(cores, "core"), plural(threads, "thread"))
    }
}

/// "1920 × 1080, 60 Hz".
pub(crate) fn resolution_label(display: &DisplayFacts) -> String {
    match display.refresh_hz.filter(|hz| *hz > 1) {
        Some(hz) => format!("{} × {}, {hz} Hz", display.width, display.height),
        None => format!("{} × {}", display.width, display.height),
    }
}

/// What the desktop looks like at the display's scale: "Looks like
/// 1536 × 864" for 1920 × 1080 at 125 %.
pub(crate) fn looks_like_label(display: &DisplayFacts) -> String {
    let scale = display.scale_percent.max(100);
    let width = (u64::from(display.width) * 100 / u64::from(scale)) as u32;
    let height = (u64::from(display.height) * 100 / u64::from(scale)) as u32;
    format!("Looks like {width} × {height}")
}

/// A placeholder some firmware leaves in its maker or model fields.
pub(crate) fn is_placeholder(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty()
        || value == "system manufacturer"
        || value == "system product name"
        || value == "default string"
        || value.contains("to be filled by o.e.m")
}

// ---- Windows -----------------------------------------------------------------

pub(crate) struct WindowsHost;

impl Host for WindowsHost {
    fn about(&self) -> AboutFacts {
        AboutFacts {
            computer_name: std::env::var("COMPUTERNAME")
                .ok()
                .filter(|name| !name.trim().is_empty()),
            model: win32::model(),
            processor: win32::processor(),
            cores: win32::cores(),
            memory: win32::memory(),
            graphics: win32::graphics(),
            os: win32::os_version(),
            drives: drives(),
        }
    }

    fn displays(&self) -> Result<Vec<DisplayFacts>, String> {
        win32::displays()
    }

    fn output_volume(&self) -> Result<VolumeReading, String> {
        win32::output_volume()
    }

    fn set_desktop_wallpaper(&self, image: &Path, fit: WallpaperFit) -> Result<(), String> {
        win32::set_desktop_wallpaper(image, fit)
    }
}

/// The fixed drives (Files' Locations list, `rmac-mounts`), with their
/// capacity; removable and optical media are left out.
fn drives() -> Vec<Drive> {
    let Ok(volumes) = rmac_mounts::volumes() else {
        return Vec::new();
    };
    volumes
        .into_iter()
        // `volumes()` puts a Unix-style "/" first; on Windows that is
        // only the current drive again.
        .filter(|volume| volume.mount.identity != "system:/" && !volume.mount.ejectable)
        .filter_map(|volume| {
            let usage = volume.usage?;
            Some(Drive {
                name: volume.mount.name,
                total: usage.total,
                available: usage.available,
            })
        })
        .collect()
}

mod win32 {
    use std::path::Path;

    use rmac_shell_settings::WallpaperFit;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Registry::{
        RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ,
        RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    };

    use super::{DisplayFacts, OsVersion, VolumeReading};

    fn registry_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let key = HSTRING::from(key);
        let value = HSTRING::from(value);
        let mut size = 0u32;
        // SAFETY: valid key/value strings; a size-only query writes `size`.
        unsafe {
            RegGetValueW(
                root,
                &key,
                &value,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
        // SAFETY: `buffer` holds `size` bytes, which `size` reports back.
        unsafe {
            RegGetValueW(
                root,
                &key,
                &value,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..length])
            .trim()
            .to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn registry_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let key = HSTRING::from(key);
        let value = HSTRING::from(value);
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `data` is a u32 and `size` says so.
        unsafe {
            RegGetValueW(
                root,
                &key,
                &value,
                RRF_RT_REG_DWORD,
                None,
                Some((&mut data as *mut u32).cast()),
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        Some(data)
    }

    const BIOS: &str = r"HARDWARE\DESCRIPTION\System\BIOS";
    const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    pub(super) fn model() -> Option<String> {
        let maker = registry_string(HKEY_LOCAL_MACHINE, BIOS, "SystemManufacturer")
            .filter(|maker| !super::is_placeholder(maker));
        let product = registry_string(HKEY_LOCAL_MACHINE, BIOS, "SystemProductName")
            .filter(|product| !super::is_placeholder(product));
        match (maker, product) {
            (Some(maker), Some(product)) if product.starts_with(&maker) => Some(product),
            (Some(maker), Some(product)) => Some(format!("{maker} {product}")),
            (None, Some(product)) => Some(product),
            (Some(maker), None) => Some(maker),
            (None, None) => None,
        }
    }

    pub(super) fn processor() -> Option<String> {
        registry_string(
            HKEY_LOCAL_MACHINE,
            r"HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
        )
        .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    pub(super) fn cores() -> Option<(usize, usize)> {
        use windows::Win32::System::SystemInformation::{
            GetLogicalProcessorInformation, RelationProcessorCore,
            SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
        };
        let mut length = 0u32;
        // SAFETY: a size query; it fails with ERROR_INSUFFICIENT_BUFFER and
        // writes the size needed.
        let _ = unsafe { GetLogicalProcessorInformation(None, &mut length) };
        let entry = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        let count = length as usize / entry;
        if count == 0 {
            return None;
        }
        let mut entries = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); count];
        // SAFETY: `entries` holds `length` bytes.
        unsafe { GetLogicalProcessorInformation(Some(entries.as_mut_ptr()), &mut length) }.ok()?;
        let filled = (length as usize / entry).min(count);
        let mut cores = 0;
        let mut threads = 0;
        for information in &entries[..filled] {
            if information.Relationship == RelationProcessorCore {
                cores += 1;
                threads += information.ProcessorMask.count_ones() as usize;
            }
        }
        (cores > 0).then_some((cores, threads))
    }

    pub(super) fn memory() -> Option<u64> {
        use windows::Win32::System::SystemInformation::{
            GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };
        let mut kilobytes = 0u64;
        // SAFETY: writes one u64.
        if unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) }.is_ok() && kilobytes > 0 {
            return Some(kilobytes * 1024);
        }
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: `status.dwLength` is set as the call requires.
        unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
        Some(status.ullTotalPhys)
    }

    pub(super) fn graphics() -> Vec<String> {
        use windows::Win32::Graphics::Gdi::{
            EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_MIRRORING_DRIVER,
        };
        let mut names: Vec<String> = Vec::new();
        for index in 0..16 {
            let mut device = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            // SAFETY: `device.cb` is set; a null device name enumerates
            // adapters.
            if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
                break;
            }
            if (device.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER).0 != 0 {
                continue;
            }
            let name = wide_text(&device.DeviceString);
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    pub(super) fn os_version() -> Option<OsVersion> {
        let product = registry_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "ProductName")?;
        Some(OsVersion {
            product,
            release: registry_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "DisplayVersion"),
            build: registry_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentBuildNumber")
                .and_then(|build| build.parse().ok()),
            revision: registry_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "UBR"),
        })
    }

    fn wide_text(buffer: &[u16]) -> String {
        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..length])
            .trim()
            .to_owned()
    }

    /// Each active display path's GDI source name ("\\.\DISPLAY1") with the
    /// monitor's friendly name and whether it is the built-in panel.
    fn display_names() -> Vec<(String, String, bool)> {
        use windows::Win32::Devices::Display::{
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
            DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
            DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
        };
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        // SAFETY: two counts out.
        if unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        }
        .is_err()
        {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        // SAFETY: the buffers hold the counts passed with them.
        if unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        }
        .is_err()
        {
            return Vec::new();
        }
        paths.truncate(path_count as usize);
        let mut names = Vec::new();
        for path in &paths {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            // SAFETY: the header names its own size and type.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
                continue;
            }
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            // SAFETY: as above.
            let named = unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0;
            let technology = path.targetInfo.outputTechnology;
            let built_in = technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED;
            let friendly = if named {
                wide_text(&target.monitorFriendlyDeviceName)
            } else {
                String::new()
            };
            names.push((wide_text(&source.viewGdiDeviceName), friendly, built_in));
        }
        names
    }

    pub(super) fn displays() -> Result<Vec<DisplayFacts>, String> {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{LPARAM, RECT};
        use windows::Win32::Graphics::Gdi::{
            EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW,
            ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        };
        use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            // SAFETY: `data` is the `Vec` passed below, alive for the call.
            unsafe { (*(data.0 as *mut Vec<HMONITOR>)).push(monitor) };
            BOOL(1)
        }

        let mut monitors: Vec<HMONITOR> = Vec::new();
        // SAFETY: `collect` only pushes onto `monitors`.
        unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize),
            )
        }
        .ok()
        .map_err(|error| format!("Windows could not list the displays ({error})."))?;
        let names = display_names();
        let mut displays = Vec::new();
        for (index, monitor) in monitors.into_iter().enumerate() {
            let mut information = MONITORINFOEXW::default();
            information.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: `cbSize` says this is the extended structure.
            if !unsafe {
                GetMonitorInfoW(
                    monitor,
                    &mut information as *mut MONITORINFOEXW as *mut MONITORINFO,
                )
            }
            .as_bool()
            {
                continue;
            }
            let device = wide_text(&information.szDevice);
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let device_name = HSTRING::from(device.as_str());
            // SAFETY: `dmSize` is set; the device name is a valid string.
            let have_mode =
                unsafe { EnumDisplaySettingsW(&device_name, ENUM_CURRENT_SETTINGS, &mut mode) }
                    .as_bool();
            let bounds = information.monitorInfo.rcMonitor;
            let (width, height, refresh) = if have_mode {
                (
                    mode.dmPelsWidth,
                    mode.dmPelsHeight,
                    Some(mode.dmDisplayFrequency),
                )
            } else {
                (
                    (bounds.right - bounds.left).max(0) as u32,
                    (bounds.bottom - bounds.top).max(0) as u32,
                    None,
                )
            };
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            // SAFETY: two u32 outs.
            let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            let named = names.iter().find(|(source, _, _)| *source == device);
            let built_in = named.is_some_and(|(_, _, built_in)| *built_in);
            let name = match named {
                Some((_, friendly, _)) if !friendly.is_empty() => friendly.clone(),
                _ if built_in => "Built-in Display".to_owned(),
                _ => format!("Display {}", index + 1),
            };
            displays.push(DisplayFacts {
                name,
                width,
                height,
                refresh_hz: refresh,
                scale_percent: (dpi_x * 100).div_ceil(96).max(100),
                primary: information.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                built_in,
            });
        }
        // The main display first, as the Mac lists it.
        displays.sort_by_key(|display| !display.primary);
        Ok(displays)
    }

    /// Core Audio's default render endpoint: its name, master volume and
    /// mute switch.
    pub(super) fn output_volume() -> Result<VolumeReading, String> {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
            STGM_READ,
        };

        // SAFETY: this thread joins the multithreaded apartment for the
        // calls below and leaves it again before returning.
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let reading = (|| -> windows::core::Result<VolumeReading> {
            // SAFETY: plain COM calls on interfaces this closure owns.
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                let level = volume.GetMasterVolumeLevelScalar()?;
                let muted = volume.GetMute()?.as_bool();
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
                    .ok()
                    .map(|value| value.to_string())
                    .filter(|name| !name.trim().is_empty());
                Ok(VolumeReading {
                    device: name,
                    percent: (level.clamp(0.0, 1.0) * 100.0).round() as u8,
                    muted,
                })
            }
        })();
        if initialized {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
        reading.map_err(|error| format!("Windows did not report an output device ({error})."))
    }

    pub(super) fn set_desktop_wallpaper(image: &Path, fit: WallpaperFit) -> Result<(), String> {
        use windows::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
        };
        // HKCU\Control Panel\Desktop's placement values, which Settings ▸
        // Personalisation ▸ Background writes too.
        let (style, tile) = match fit {
            WallpaperFit::Fill => ("10", "0"),
            WallpaperFit::Fit => ("6", "0"),
            WallpaperFit::Stretch => ("2", "0"),
            WallpaperFit::Center => ("0", "0"),
            WallpaperFit::Tile => ("0", "1"),
        };
        for (name, value) in [("WallpaperStyle", style), ("TileWallpaper", tile)] {
            let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `data` is a terminated wide string of the size given.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(r"Control Panel\Desktop"),
                    &HSTRING::from(name),
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            }
            .ok()
            .map_err(|error| format!("Windows did not take the wallpaper placement ({error})."))?;
        }
        let mut path: Vec<u16> = image.as_os_str().encode_wide_terminated();
        // SAFETY: `path` is a terminated wide string that outlives the call.
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(path.as_mut_ptr().cast()),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|error| format!("Windows did not set the desktop picture ({error})."))
    }

    trait EncodeWideTerminated {
        fn encode_wide_terminated(&self) -> Vec<u16>;
    }

    impl EncodeWideTerminated for std::ffi::OsStr {
        fn encode_wide_terminated(&self) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt as _;
            self.encode_wide().chain(Some(0)).collect()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Fixed facts for pane tests.
    pub(crate) struct FakeHost;

    impl Host for FakeHost {
        fn about(&self) -> AboutFacts {
            AboutFacts {
                computer_name: Some("LULO-PC".into()),
                model: Some("HP Laptop 15s".into()),
                processor: Some("AMD Ryzen 3 7320U with Radeon Graphics".into()),
                cores: Some((4, 8)),
                memory: Some(8 << 30),
                graphics: vec!["AMD Radeon(TM) Graphics".into()],
                os: Some(OsVersion {
                    product: "Windows 10 Home".into(),
                    release: Some("25H2".into()),
                    build: Some(26_200),
                    revision: Some(6584),
                }),
                drives: vec![Drive {
                    name: "Local Disk (C:)".into(),
                    total: 254_000_000_000,
                    available: 31_000_000_000,
                }],
            }
        }

        fn displays(&self) -> Result<Vec<DisplayFacts>, String> {
            Ok(vec![DisplayFacts {
                name: "Built-in Display".into(),
                width: 1920,
                height: 1080,
                refresh_hz: Some(60),
                scale_percent: 125,
                primary: true,
                built_in: true,
            }])
        }

        fn output_volume(&self) -> Result<VolumeReading, String> {
            Ok(VolumeReading {
                device: Some("Speakers".into()),
                percent: 42,
                muted: false,
            })
        }

        fn set_desktop_wallpaper(&self, _: &Path, _: WallpaperFit) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn windows_11_is_named_from_its_build_number() {
        let os = FakeHost.about().os.unwrap();
        assert_eq!(os_label(&os), "Windows 11 Home 25H2 (build 26200.6584)");
        let ten = OsVersion {
            product: "Windows 10 Pro".into(),
            release: Some("22H2".into()),
            build: Some(19_045),
            revision: None,
        };
        assert_eq!(os_label(&ten), "Windows 10 Pro 22H2 (build 19045)");
    }

    #[test]
    fn memory_and_capacity_read_like_the_mac() {
        assert_eq!(memory_label(8 << 30), "8 GB");
        assert_eq!(memory_label(7_700_000_000), "7 GB");
        assert_eq!(memory_label(512 << 20), "512 MB");
        assert_eq!(capacity_label(254_000_000_000), "254.00 GB");
        assert_eq!(capacity_label(31_456_000_000), "31.46 GB");
        assert_eq!(capacity_label(2_000_000_000_000), "2.00 TB");
        assert_eq!(capacity_label(12), "12 bytes");
    }

    #[test]
    fn cores_and_threads_collapse_when_equal() {
        assert_eq!(cores_label((4, 8)), "4 cores, 8 threads");
        assert_eq!(cores_label((2, 2)), "2 cores");
        assert_eq!(cores_label((1, 1)), "1 core");
    }

    #[test]
    fn display_labels_show_resolution_and_the_scaled_size() {
        let display = FakeHost.displays().unwrap().remove(0);
        assert_eq!(resolution_label(&display), "1920 × 1080, 60 Hz");
        assert_eq!(looks_like_label(&display), "Looks like 1536 × 864");
    }

    #[test]
    fn firmware_placeholders_are_not_shown_as_a_model() {
        assert!(is_placeholder("System manufacturer"));
        assert!(is_placeholder("To Be Filled By O.E.M."));
        assert!(!is_placeholder("HP"));
    }

    #[test]
    fn the_real_host_answers_without_failing() {
        // On the CI runner and the laptop alike: every fact is optional,
        // but the calls must not panic, and Windows always has an OS
        // version and at least one display.
        let about = WindowsHost.about();
        assert!(about.os.is_some());
        assert!(!WindowsHost.displays().unwrap().is_empty());
        let _ = WindowsHost.output_volume();
    }
}
