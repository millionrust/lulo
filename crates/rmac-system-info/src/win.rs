//! Windows facts, read from the registry and Win32. Read on snapshot only:
//! Windows has no change notification for these, and none of them change
//! while the PC runs except the name, which needs a restart anyway.

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_MIRRORING_DRIVER,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::SystemInformation::{
    ComputerNameDnsHostname, ComputerNamePhysicalDnsHostname, GetComputerNameExW,
    GetNativeSystemInfo, GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx,
    COMPUTER_NAME_FORMAT, MEMORYSTATUSEX, PROCESSOR_ARCHITECTURE, PROCESSOR_ARCHITECTURE_AMD64,
    PROCESSOR_ARCHITECTURE_ARM64, PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
};
use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

use crate::facts::{architecture_label, format_memory_bytes, safe_fact};
use crate::{Error, ErrorKind, Snapshot};

const BIOS: &str = r"HARDWARE\DESCRIPTION\System\BIOS";
const CPU0: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const MAX_GRAPHICS_DEVICES: u32 = 16;

pub(crate) fn system_snapshot() -> Snapshot {
    let hostname = computer_name(ComputerNameDnsHostname)
        .or_else(|| std::env::var("COMPUTERNAME").ok().and_then(safe_fact))
        .unwrap_or_else(|| "PC".into());
    let version = |value: &str| registry_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, value);
    let build = version("CurrentBuildNumber").and_then(|build| build.parse::<u32>().ok());
    let version_number = |value: &str| registry_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, value);
    Snapshot {
        hostname,
        static_hostname: computer_name(ComputerNamePhysicalDnsHostname),
        pretty_hostname: None,
        hostname_mutable: false,
        hostname_unavailable_reason: Some(
            "Renaming this PC needs an administrator; use Windows Settings.".into(),
        ),
        operating_system: os_label(
            version("ProductName").as_deref(),
            version("DisplayVersion").as_deref(),
            build,
        ),
        kernel: kernel_label(
            version_number("CurrentMajorVersionNumber"),
            version_number("CurrentMinorVersionNumber"),
            version("CurrentVersion").as_deref(),
            build,
            version_number("UBR"),
        ),
        architecture: architecture_label(native_architecture()).into(),
        hardware_vendor: firmware_string("SystemManufacturer"),
        hardware_model: firmware_string("SystemProductName"),
        processor: registry_string(HKEY_LOCAL_MACHINE, CPU0, "ProcessorNameString")
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .and_then(safe_fact),
        memory: installed_memory().map(format_memory_bytes),
        graphics: graphics(),
        session: Some(session_label(remote_session()).into()),
        desktop: Some(desktop_label(lulo_session_running()).into()),
    }
}

pub(crate) fn system_set_static_hostname(_hostname: &str) -> Result<(), Error> {
    // SetComputerNameExW needs an elevated process and only takes effect
    // after a restart, so Lulo leaves renaming to Windows Settings.
    Err(Error::new(
        ErrorKind::Authorization,
        "set the hostname",
        "renaming this PC needs an administrator and a restart; use Windows Settings",
    ))
}

/// "Windows 11 Pro 24H2". Windows 11 keeps `ProductName` at "Windows 10 …";
/// its builds start at 22000.
fn os_label(product: Option<&str>, release: Option<&str>, build: Option<u32>) -> String {
    let mut label = product
        .map(str::trim)
        .filter(|product| !product.is_empty())
        .unwrap_or("Windows")
        .to_owned();
    if build.is_some_and(|build| build >= 22_000) && label.starts_with("Windows 10") {
        label = label.replacen("Windows 10", "Windows 11", 1);
    }
    if let Some(release) = release.map(str::trim).filter(|release| !release.is_empty()) {
        label.push(' ');
        label.push_str(release);
    }
    label
}

/// "NT 10.0.26100.6584". Before Windows 10 the registry has no major and
/// minor numbers, only `CurrentVersion` ("6.3").
fn kernel_label(
    major: Option<u32>,
    minor: Option<u32>,
    current_version: Option<&str>,
    build: Option<u32>,
    revision: Option<u32>,
) -> String {
    let mut version = match (major, minor) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        _ => current_version
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .unwrap_or("unknown")
            .to_owned(),
    };
    if let Some(build) = build {
        version.push_str(&format!(".{build}"));
        if let Some(revision) = revision {
            version.push_str(&format!(".{revision}"));
        }
    }
    format!("NT {version}")
}

/// The machine's own architecture, named as `uname -m` would, so an x64
/// build running emulated on ARM still says ARM64.
fn architecture_name(architecture: PROCESSOR_ARCHITECTURE) -> &'static str {
    match architecture {
        PROCESSOR_ARCHITECTURE_AMD64 => "x86_64",
        PROCESSOR_ARCHITECTURE_ARM64 => "aarch64",
        PROCESSOR_ARCHITECTURE_INTEL => "x86",
        _ => std::env::consts::ARCH,
    }
}

fn native_architecture() -> &'static str {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: fills the struct it is given.
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: GetNativeSystemInfo always fills the architecture half of the
    // union.
    architecture_name(unsafe { info.Anonymous.Anonymous.wProcessorArchitecture })
}

/// A placeholder some firmware leaves in its maker or model fields.
fn is_placeholder(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty()
        || value == "system manufacturer"
        || value == "system product name"
        || value == "default string"
        || value == "not applicable"
        || value.contains("to be filled by o.e.m")
}

fn firmware_string(value: &str) -> Option<String> {
    registry_string(HKEY_LOCAL_MACHINE, BIOS, value)
        .filter(|text| !is_placeholder(text))
        .and_then(safe_fact)
}

fn session_label(remote: bool) -> &'static str {
    if remote {
        "Remote Desktop"
    } else {
        "Console"
    }
}

fn desktop_label(lulo_running: bool) -> &'static str {
    if lulo_running {
        "Lulo"
    } else {
        "Windows"
    }
}

fn remote_session() -> bool {
    // SAFETY: a plain metric query.
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// Whether the Lulo layer is on: `lulo-session` holds this mutex for its
/// whole life (crates/rmac-win-shell/src/win/session.rs).
fn lulo_session_running() -> bool {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let name = HSTRING::from(format!(r"Local\lulo-session-{user}"));
    // SAFETY: opens the mutex only to learn that it exists, then closes it.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) } {
        Ok(mutex) => {
            // SAFETY: the handle was opened above.
            let _ = unsafe { CloseHandle(mutex) };
            true
        }
        Err(_) => false,
    }
}

fn computer_name(format: COMPUTER_NAME_FORMAT) -> Option<String> {
    let mut length = 0u32;
    // SAFETY: a size query; it fails with ERROR_MORE_DATA and writes the
    // length needed, in characters with the terminator.
    let _ = unsafe { GetComputerNameExW(format, None, &mut length) };
    if length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: `buffer` holds `length` characters; on success `length` is
    // the name's length without the terminator.
    unsafe { GetComputerNameExW(format, Some(PWSTR(buffer.as_mut_ptr())), &mut length) }.ok()?;
    let length = (length as usize).min(buffer.len());
    safe_fact(String::from_utf16_lossy(&buffer[..length]))
}

fn installed_memory() -> Option<u64> {
    let mut kilobytes = 0u64;
    // SAFETY: writes one u64.
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) }.is_ok() && kilobytes > 0 {
        return Some(kilobytes.saturating_mul(1024));
    }
    // Some virtual machines' firmware has no memory table; fall back to
    // what Windows can use.
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status.dwLength` is set as the call requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    (status.ullTotalPhys > 0).then_some(status.ullTotalPhys)
}

/// The display adapters, once each, joined as the Linux list is.
fn graphics() -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for index in 0..MAX_GRAPHICS_DEVICES {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: `device.cb` is set; a null device name enumerates adapters.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
            break;
        }
        if (device.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER).0 != 0 {
            continue;
        }
        if let Some(name) = safe_fact(wide_text(&device.DeviceString)) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    safe_fact(names.join(" · "))
}

fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

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
    safe_fact(wide_text(&buffer))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_11_is_named_from_its_build_number() {
        assert_eq!(
            os_label(Some("Windows 10 Pro"), Some("24H2"), Some(26_100)),
            "Windows 11 Pro 24H2"
        );
        assert_eq!(
            os_label(Some("Windows 10 Home"), Some("22H2"), Some(19_045)),
            "Windows 10 Home 22H2"
        );
        assert_eq!(os_label(None, None, None), "Windows");
    }

    #[test]
    fn kernel_reads_as_the_nt_version() {
        assert_eq!(
            kernel_label(Some(10), Some(0), Some("6.3"), Some(26_100), Some(6584)),
            "NT 10.0.26100.6584"
        );
        assert_eq!(
            kernel_label(None, None, Some("6.3"), Some(9600), None),
            "NT 6.3.9600"
        );
    }

    #[test]
    fn architectures_are_named_like_uname() {
        assert_eq!(
            architecture_label(architecture_name(PROCESSOR_ARCHITECTURE_AMD64)),
            "x86_64"
        );
        assert_eq!(
            architecture_label(architecture_name(PROCESSOR_ARCHITECTURE_ARM64)),
            "ARM64"
        );
    }

    #[test]
    fn firmware_placeholders_are_not_facts() {
        for placeholder in [
            "To be filled by O.E.M.",
            "System Product Name",
            "Default string",
        ] {
            assert!(is_placeholder(placeholder), "{placeholder}");
        }
        assert!(!is_placeholder("HP"));
    }

    #[test]
    fn the_real_host_answers() {
        let snapshot = system_snapshot();
        assert!(snapshot.operating_system.starts_with("Windows"));
        assert!(snapshot.kernel.starts_with("NT "));
        assert!(!snapshot.hostname.is_empty());
        assert_eq!(
            system_set_static_hostname("lulo").unwrap_err().kind(),
            ErrorKind::Authorization
        );
    }
}
