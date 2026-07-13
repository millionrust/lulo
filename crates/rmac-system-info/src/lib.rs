//! Privacy-safe platform identity and hostname administration.
//!
//! The snapshot intentionally excludes usernames, machine IDs, serial numbers,
//! network addresses, and paths. Linux hostname changes go directly through
//! systemd-hostnamed so the platform can provide interactive polkit authority.

use std::fmt;
#[cfg(not(target_os = "macos"))]
use std::fs;
use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub hostname: String,
    pub static_hostname: Option<String>,
    pub pretty_hostname: Option<String>,
    pub hostname_mutable: bool,
    pub hostname_unavailable_reason: Option<String>,
    pub operating_system: String,
    pub kernel: String,
    pub architecture: String,
    pub hardware_vendor: Option<String>,
    pub hardware_model: Option<String>,
    pub processor: Option<String>,
    pub memory: Option<String>,
    pub session: Option<String>,
    pub desktop: Option<String>,
}

impl Snapshot {
    pub fn display_hostname(&self) -> &str {
        self.static_hostname
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.hostname)
    }

    /// A deliberately bounded report suitable for the clipboard or a bug.
    pub fn diagnostic_report(&self, graphics: Option<&str>) -> String {
        let mut lines = vec![
            "rmac system report".to_string(),
            format!("Operating system: {}", self.operating_system),
            format!("Kernel: {}", self.kernel),
            format!("Architecture: {}", self.architecture),
        ];
        push_fact(
            &mut lines,
            "Hardware vendor",
            self.hardware_vendor.as_deref(),
        );
        push_fact(&mut lines, "Hardware model", self.hardware_model.as_deref());
        push_fact(&mut lines, "Processor", self.processor.as_deref());
        push_fact(&mut lines, "Memory", self.memory.as_deref());
        push_fact(&mut lines, "Graphics", graphics);
        push_fact(&mut lines, "Session", self.session.as_deref());
        push_fact(&mut lines, "Desktop", self.desktop.as_deref());
        lines.join("\n") + "\n"
    }
}

fn push_fact(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        lines.push(format!("{label}: {value}"));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidName,
    Unavailable,
    Authorization,
    Mutation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(kind: ErrorKind, operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            operation,
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Injectable boundary used by the UI and fixture-backed consumers.
pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(system_snapshot())
    }

    fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error> {
        let hostname = normalize_static_hostname(hostname)?;
        system_set_static_hostname(&hostname)?;
        self.snapshot()
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_static_hostname(hostname: &str) -> Result<Snapshot, Error> {
    SystemService.set_static_hostname(hostname)
}

pub fn validate_static_hostname(hostname: &str) -> Result<(), Error> {
    normalize_static_hostname(hostname).map(drop)
}

fn normalize_static_hostname(hostname: &str) -> Result<String, Error> {
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

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Snapshot {
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
        processor: fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|contents| processor_from_cpuinfo(&contents)),
        memory: fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|contents| memory_from_meminfo(&contents)),
        session: environment_value("XDG_SESSION_TYPE"),
        desktop: environment_value("XDG_CURRENT_DESKTOP"),
    }
}

#[cfg(target_os = "macos")]
fn system_snapshot() -> Snapshot {
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
    let Ok(connection) = zbus::blocking::Connection::system() else {
        return unavailable("The system hostname service is unavailable.");
    };
    let Ok(proxy) = hostname_proxy(&connection) else {
        return unavailable("The system hostname service is unavailable.");
    };
    let Some(current) = optional_property::<String>(&proxy, "Hostname") else {
        return unavailable("The system hostname service is unavailable.");
    };
    HostnameSnapshot {
        current,
        static_name: optional_property(&proxy, "StaticHostname"),
        pretty: optional_property(&proxy, "PrettyHostname"),
        mutable: true,
        unavailable_reason: None,
        kernel_name: optional_property(&proxy, "KernelName"),
        kernel_release: optional_property(&proxy, "KernelRelease"),
    }
}

#[cfg(not(target_os = "macos"))]
fn system_set_static_hostname(hostname: &str) -> Result<(), Error> {
    let connection = zbus::blocking::Connection::system().map_err(|_| {
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
fn system_set_static_hostname(_hostname: &str) -> Result<(), Error> {
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

fn command(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|output| !output.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn environment_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn read_os_release(key: &str) -> Option<String> {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| os_release_value(&contents, key))
}

#[cfg(any(not(target_os = "macos"), test))]
fn os_release_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key).then(|| value.trim().trim_matches(['\'', '"']).to_string())
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn colon_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        (candidate.trim() == key).then(|| value.trim().to_string())
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn processor_from_cpuinfo(contents: &str) -> Option<String> {
    ["model name", "Hardware", "Processor"]
        .into_iter()
        .find_map(|key| colon_value(contents, key))
}

#[cfg(any(not(target_os = "macos"), test))]
fn memory_from_meminfo(contents: &str) -> Option<String> {
    colon_value(contents, "MemTotal")
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kibibytes| format_memory_bytes(kibibytes.saturating_mul(1024)))
}

fn format_memory_bytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

fn architecture_label(architecture: &str) -> &str {
    match architecture {
        "aarch64" => "ARM64",
        "x86_64" => "x86_64",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeService {
        snapshot: Snapshot,
        mutation: Result<(), Error>,
    }

    impl Service for FakeService {
        fn snapshot(&self) -> Result<Snapshot, Error> {
            Ok(self.snapshot.clone())
        }

        fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error> {
            let hostname = normalize_static_hostname(hostname)?;
            self.mutation.clone()?;
            let mut snapshot = self.snapshot.clone();
            snapshot.hostname = hostname.clone();
            snapshot.static_hostname = Some(hostname);
            Ok(snapshot)
        }
    }

    #[test]
    fn hostname_validation_matches_static_hostname_constraints() {
        for valid in ["rmac", "studio-pc", "A1"] {
            assert!(validate_static_hostname(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "has space", "-leading", "trailing-", "two.labels"] {
            assert_eq!(
                validate_static_hostname(invalid).unwrap_err().kind(),
                ErrorKind::InvalidName,
                "{invalid}"
            );
        }
        assert!(validate_static_hostname(&"a".repeat(64)).is_err());
        assert_eq!(normalize_static_hostname("Studio-PC").unwrap(), "studio-pc");
    }

    #[test]
    fn fake_service_returns_refreshed_authoritative_hostname() {
        let service = FakeService {
            snapshot: Snapshot {
                hostname: "before".into(),
                operating_system: "Ubuntu".into(),
                ..Snapshot::default()
            },
            mutation: Ok(()),
        };

        let snapshot = service.set_static_hostname("after").unwrap();

        assert_eq!(snapshot.hostname, "after");
        assert_eq!(snapshot.static_hostname.as_deref(), Some("after"));
    }

    #[test]
    fn display_hostname_prefers_the_authoritative_static_value() {
        let snapshot = Snapshot {
            hostname: "transient".into(),
            static_hostname: Some("static-name".into()),
            pretty_hostname: Some("Unrelated Pretty Name".into()),
            ..Snapshot::default()
        };

        assert_eq!(snapshot.display_hostname(), "static-name");
    }

    #[test]
    fn failed_fake_mutation_does_not_replace_last_known_good_state() {
        let before = Snapshot {
            hostname: "before".into(),
            static_hostname: Some("before".into()),
            ..Snapshot::default()
        };
        let service = FakeService {
            snapshot: before.clone(),
            mutation: Err(Error::new(
                ErrorKind::Authorization,
                "set the hostname",
                "authorization was cancelled",
            )),
        };

        assert!(service.set_static_hostname("after").is_err());
        assert_eq!(service.snapshot().unwrap(), before);
    }

    #[test]
    fn diagnostics_exclude_identity_and_serial_data() {
        let snapshot = Snapshot {
            hostname: "private-host".into(),
            static_hostname: Some("private-host".into()),
            operating_system: "Ubuntu 26.04 LTS".into(),
            kernel: "Linux 6.18".into(),
            architecture: "x86_64".into(),
            processor: Some("Example CPU".into()),
            ..Snapshot::default()
        };

        let report = snapshot.diagnostic_report(Some("Example GPU"));

        assert!(report.contains("Ubuntu 26.04 LTS"));
        assert!(report.contains("Example GPU"));
        assert!(!report.contains("private-host"));
        assert!(!report.to_ascii_lowercase().contains("serial"));
        assert!(!report.to_ascii_lowercase().contains("user"));
    }

    #[test]
    fn standard_linux_fact_files_are_parsed_without_private_fields() {
        assert_eq!(
            os_release_value(
                "NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\n",
                "PRETTY_NAME"
            )
            .as_deref(),
            Some("Ubuntu 26.04 LTS")
        );
        assert_eq!(
            processor_from_cpuinfo("processor : 0\nmodel name : Example CPU\n").as_deref(),
            Some("Example CPU")
        );
        assert_eq!(
            memory_from_meminfo("MemTotal:       16777216 kB\n").as_deref(),
            Some("17.2 GB")
        );
    }
}
