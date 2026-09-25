use crate::facts::*;
use crate::{Error, ErrorKind, Snapshot};

#[cfg(not(target_os = "macos"))]
const MAX_FACT_FILE_BYTES: u64 = 1024 * 1024;

pub(crate) fn normalize_static_hostname(hostname: &str) -> Result<String, Error> {
    if hostname.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidName,
            "validate the hostname",
            "enter a hostname",
        ));
    }
    if hostname.len() > 63 {
        return Err(Error::new(
            ErrorKind::InvalidName,
            "validate the hostname",
            "the hostname must be 63 bytes or fewer",
        ));
    }
    let hostname = hostname.to_ascii_lowercase();
    if !hostname
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::new(
            ErrorKind::InvalidName,
            "validate the hostname",
            "use only letters, numbers, and hyphens",
        ));
    }
    if hostname.starts_with('-') || hostname.ends_with('-') {
        return Err(Error::new(
            ErrorKind::InvalidName,
            "validate the hostname",
            "the hostname must begin and end with a letter or number",
        ));
    }
    Ok(hostname)
}

pub(crate) fn verify_static_hostname(
    snapshot: Snapshot,
    expected: &str,
) -> Result<Snapshot, Error> {
    if snapshot.static_hostname.as_deref() != Some(expected) {
        return Err(Error::new(
            ErrorKind::Mutation,
            "verify the hostname",
            "the hostname service did not report the requested static hostname",
        ));
    }
    Ok(snapshot)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn system_snapshot() -> Snapshot {
    let host = linux_hostname();
    let operating_system = read_os_release("PRETTY_NAME").unwrap_or_else(|| "Linux".into());
    let kernel_name = host
        .kernel_name
        .or_else(|| command("uname", &["-s"]))
        .unwrap_or_else(|| "Linux".into());
    let kernel_release = host
        .kernel_release
        .or_else(|| command("uname", &["-r"]))
        .unwrap_or_else(|| "unknown".into());
    Snapshot {
        hostname: host.current,
        static_hostname: host.static_name,
        pretty_hostname: host.pretty,
        hostname_mutable: host.mutable,
        hostname_unavailable_reason: host.unavailable_reason,
        operating_system,
        kernel: format!("{kernel_name} {kernel_release}"),
        architecture: architecture_label(std::env::consts::ARCH).into(),
        hardware_vendor: read_trimmed("/sys/class/dmi/id/sys_vendor"),
        hardware_model: read_trimmed("/sys/class/dmi/id/product_name"),
        processor: read_bounded("/proc/cpuinfo", MAX_FACT_FILE_BYTES)
            .and_then(|contents| processor_from_cpuinfo(&contents)),
        memory: read_bounded("/proc/meminfo", 64 * 1024)
            .and_then(|contents| memory_from_meminfo(&contents)),
        graphics: linux_graphics(),
        session: environment_value("XDG_SESSION_TYPE"),
        desktop: environment_value("XDG_CURRENT_DESKTOP"),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn system_snapshot() -> Snapshot {
    let hostname = command("scutil", &["--get", "ComputerName"])
        .or_else(|| command("hostname", &[]))
        .unwrap_or_else(|| "Mac".into());
    let name = command("sw_vers", &["-productName"]).unwrap_or_else(|| "macOS".into());
    let version = command("sw_vers", &["-productVersion"]).unwrap_or_default();
    Snapshot {
        hostname: hostname.clone(),
        pretty_hostname: Some(hostname),
        hostname_unavailable_reason: Some(
            "Hostname changes are available in the supported Linux session.".into(),
        ),
        operating_system: format!("{name} {version}").trim().to_string(),
        kernel: format!(
            "Darwin {}",
            command("uname", &["-r"]).unwrap_or_else(|| "unknown".into())
        ),
        architecture: architecture_label(std::env::consts::ARCH).into(),
        hardware_model: command("sysctl", &["-n", "hw.model"]),
        processor: command("sysctl", &["-n", "machdep.cpu.brand_string"]),
        memory: command("sysctl", &["-n", "hw.memsize"])
            .and_then(|value| value.parse::<u64>().ok())
            .map(format_memory_bytes),
        graphics: command_text("system_profiler", &["SPDisplaysDataType"])
            .and_then(|value| colon_value(&value, "Chipset Model")),
        session: Some("Aqua".into()),
        desktop: Some("macOS".into()),
        ..Snapshot::default()
    }
}

#[cfg(not(target_os = "macos"))]
struct HostnameSnapshot {
    current: String,
    static_name: Option<String>,
    pretty: Option<String>,
    mutable: bool,
    unavailable_reason: Option<String>,
    kernel_name: Option<String>,
    kernel_release: Option<String>,
}

#[cfg(not(target_os = "macos"))]
fn linux_hostname() -> HostnameSnapshot {
    let fallback = read_trimmed("/proc/sys/kernel/hostname").unwrap_or_else(|| "Linux".into());
    let unavailable = |reason: &str| HostnameSnapshot {
        current: fallback.clone(),
        static_name: read_trimmed("/etc/hostname"),
        pretty: None,
        mutable: false,
        unavailable_reason: Some(reason.into()),
        kernel_name: None,
        kernel_release: None,
    };
    let Ok(connection) = rmac_dbus::system_blocking() else {
        return unavailable("The system hostname service is unavailable.");
    };
    let Ok(proxy) = hostname_proxy(&connection) else {
        return unavailable("The system hostname service is unavailable.");
    };
    let Some(current) = optional_string_property(&proxy, "Hostname") else {
        return unavailable("The system hostname service is unavailable.");
    };
    HostnameSnapshot {
        current,
        static_name: optional_string_property(&proxy, "StaticHostname"),
        pretty: optional_string_property(&proxy, "PrettyHostname"),
        mutable: true,
        unavailable_reason: None,
        kernel_name: optional_string_property(&proxy, "KernelName"),
        kernel_release: optional_string_property(&proxy, "KernelRelease"),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn system_set_static_hostname(hostname: &str) -> Result<(), Error> {
    let connection = rmac_dbus::system_blocking().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "connect to the system hostname service",
            "the service is unavailable",
        )
    })?;
    let proxy = hostname_proxy(&connection).map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "open the system hostname service",
            "the service is unavailable",
        )
    })?;
    proxy
        .call::<_, _, ()>("SetStaticHostname", &(hostname, true))
        .map_err(hostname_mutation_error)
}

#[cfg(target_os = "macos")]
pub(crate) fn system_set_static_hostname(_hostname: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "set the hostname",
        "hostname changes are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "macos"))]
fn hostname_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, zbus::Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.hostname1",
        "/org/freedesktop/hostname1",
        "org.freedesktop.hostname1",
    )
}

#[cfg(not(target_os = "macos"))]
fn hostname_mutation_error(error: zbus::Error) -> Error {
    let detail = error.to_string();
    let lowercase = detail.to_ascii_lowercase();
    if lowercase.contains("accessdenied")
        || lowercase.contains("not authorized")
        || lowercase.contains("authentication")
        || lowercase.contains("polkit")
        || lowercase.contains("policykit")
    {
        Error::new(
            ErrorKind::Authorization,
            "set the hostname",
            "authorization was denied or cancelled",
        )
    } else {
        Error::new(ErrorKind::Mutation, "set the hostname", detail)
    }
}

#[cfg(not(target_os = "macos"))]
fn optional_property<T>(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy.get_property::<T>(name).ok()
}

#[cfg(not(target_os = "macos"))]
fn optional_string_property(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Option<String> {
    optional_property::<String>(proxy, name).and_then(safe_fact)
}
