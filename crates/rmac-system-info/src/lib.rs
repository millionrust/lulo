//! Privacy-safe platform identity and hostname administration.
//!
//! The snapshot intentionally excludes usernames, machine IDs, serial numbers,
//! network addresses, and paths. Linux hostname changes go directly through
//! systemd-hostnamed so the platform can provide interactive polkit authority.

use std::fmt;
#[cfg(not(target_os = "macos"))]
use std::fs;
use std::io::Read;
#[cfg(not(target_os = "macos"))]
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

const MAX_FACT_BYTES: usize = 4096;
#[cfg(not(target_os = "macos"))]
const MAX_FACT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const MAX_GRAPHICS_DEVICES: usize = 16;
#[cfg(not(target_os = "macos"))]
const MAX_DRM_ENTRIES: usize = 128;

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
    pub graphics: Option<String>,
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
    pub fn diagnostic_report(&self) -> String {
        let mut lines = vec!["rmac system report".to_string()];
        push_fact(&mut lines, "Operating system", Some(&self.operating_system));
        push_fact(&mut lines, "Kernel", Some(&self.kernel));
        push_fact(&mut lines, "Architecture", Some(&self.architecture));
        push_fact(
            &mut lines,
            "Hardware vendor",
            self.hardware_vendor.as_deref(),
        );
        push_fact(&mut lines, "Hardware model", self.hardware_model.as_deref());
        push_fact(&mut lines, "Processor", self.processor.as_deref());
        push_fact(&mut lines, "Memory", self.memory.as_deref());
        push_fact(&mut lines, "Graphics", self.graphics.as_deref());
        push_fact(&mut lines, "Session", self.session.as_deref());
        push_fact(&mut lines, "Desktop", self.desktop.as_deref());
        lines.join("\n") + "\n"
    }
}

fn push_fact(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| valid_fact(value)) {
        lines.push(format!("{label}: {value}"));
    }
}

fn valid_fact(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= MAX_FACT_BYTES && !value.chars().any(char::is_control)
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
        let snapshot = self.snapshot()?;
        verify_static_hostname(snapshot, &hostname)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "watch system information",
            "the system hostname event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/org/freedesktop/hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname event path"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| watch_protocol_error("invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| watch_protocol_error("invalid properties signal"))?
        .add_arg("org.freedesktop.hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname interface filter"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| watch_protocol_error("invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| watch_protocol_error("invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| watch_protocol_error("invalid owner-change signal"))?
        .add_arg("org.freedesktop.hostname1")
        .map_err(|_| watch_protocol_error("invalid hostname owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(8))
        .await
        .map_err(|_| watch_protocol_error("could not watch hostname changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| watch_protocol_error("could not watch hostname service restarts"))?
        .fuse();

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = properties.next() => {
                message
                    .ok_or_else(|| watch_protocol_error("hostname event stream ended"))?
                    .map_err(|_| watch_protocol_error("hostname event stream failed"))?;
                WatchEvent::Changed
            },
            message = owners.next() => owner_watch_event(message)?,
            _ = closed => return Ok(()),
        };
        let _ = sender.try_send(event);
    }
}

#[cfg(target_os = "linux")]
fn owner_watch_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<WatchEvent, Error> {
    let message = message
        .ok_or_else(|| watch_protocol_error("hostname owner stream ended"))?
        .map_err(|_| watch_protocol_error("hostname owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| watch_protocol_error("invalid hostname owner change"))?;
    owner_change_event(&name, &new_owner)
        .ok_or_else(|| watch_protocol_error("unexpected hostname owner change"))
}

#[cfg(any(target_os = "linux", test))]
fn owner_change_event(name: &str, new_owner: &str) -> Option<WatchEvent> {
    (name == "org.freedesktop.hostname1").then_some(if new_owner.is_empty() {
        WatchEvent::Unavailable
    } else {
        WatchEvent::Changed
    })
}

#[cfg(target_os = "linux")]
fn watch_protocol_error(detail: &'static str) -> Error {
    Error::new(ErrorKind::Unavailable, "watch system information", detail)
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

fn verify_static_hostname(snapshot: Snapshot, expected: &str) -> Result<Snapshot, Error> {
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
    let Ok(connection) = zbus::blocking::Connection::system() else {
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

#[cfg(not(target_os = "macos"))]
fn optional_string_property(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Option<String> {
    optional_property::<String>(proxy, name).and_then(safe_fact)
}

fn command(program: &str, args: &[&str]) -> Option<String> {
    command_text(program, args).and_then(safe_fact)
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    let output = bounded_command_output(&mut command)?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    read_bounded(path, 64 * 1024).and_then(safe_fact)
}

#[cfg(not(target_os = "macos"))]
fn environment_value(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(safe_fact)
}

#[cfg(not(target_os = "macos"))]
fn read_os_release(key: &str) -> Option<String> {
    read_bounded("/etc/os-release", 64 * 1024).and_then(|contents| os_release_value(&contents, key))
}

#[cfg(not(target_os = "macos"))]
fn read_bounded(path: impl AsRef<Path>, limit: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn safe_fact(value: String) -> Option<String> {
    let value = value.trim();
    valid_fact(value).then(|| value.to_string())
}

#[cfg(any(not(target_os = "macos"), test))]
fn os_release_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key)
            .then(|| value.trim().trim_matches(['\'', '"']).to_string())
            .and_then(safe_fact)
    })
}

fn colon_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        (candidate.trim() == key)
            .then(|| value.trim().to_string())
            .and_then(safe_fact)
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

struct BoundedCommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
}

fn bounded_command_output(command: &mut Command) -> Option<BoundedCommandOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));
    let deadline = std::time::Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return None;
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_reader.join().ok()?.ok()?;
    let (_, stderr_truncated) = stderr_reader.join().ok()?.ok()?;
    if stdout_truncated || stderr_truncated {
        return None;
    }
    Some(BoundedCommandOutput { status, stdout })
}

fn drain_bounded(mut reader: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_COMMAND_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MAX_COMMAND_OUTPUT_BYTES;
    if truncated {
        bytes.truncate(MAX_COMMAND_OUTPUT_BYTES);
    }
    Ok((bytes, truncated))
}

#[cfg(not(target_os = "macos"))]
fn linux_graphics() -> Option<String> {
    let mut devices = fs::read_dir("/sys/class/drm")
        .ok()?
        .take(MAX_DRM_ENTRIES)
        .filter_map(Result::ok)
        .filter(|entry| is_drm_card_name(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| linux_graphics_device(&entry.path()))
        .collect::<Vec<_>>();
    devices.sort();
    devices.dedup();
    devices.truncate(MAX_GRAPHICS_DEVICES);
    safe_fact(devices.join(" · "))
}

#[cfg(not(target_os = "macos"))]
fn linux_graphics_device(card: &Path) -> Option<String> {
    let device = card.join("device");
    let driver = fs::read_link(device.join("driver"))
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .and_then(safe_fact);
    let vendor = read_trimmed(device.join("vendor")).and_then(normalize_pci_id);
    let product = read_trimmed(device.join("device")).and_then(normalize_pci_id);
    graphics_label(driver.as_deref(), vendor.as_deref(), product.as_deref())
}

#[cfg(any(not(target_os = "macos"), test))]
fn is_drm_card_name(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn normalize_pci_id(value: String) -> Option<String> {
    let value = value.strip_prefix("0x").unwrap_or(&value);
    (value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

#[cfg(any(not(target_os = "macos"), test))]
fn graphics_label(
    driver: Option<&str>,
    vendor: Option<&str>,
    product: Option<&str>,
) -> Option<String> {
    let vendor_name = match vendor {
        Some("1002") => Some("AMD"),
        Some("10de") => Some("NVIDIA"),
        Some("106b") => Some("Apple"),
        Some("1234") => Some("QEMU"),
        Some("1af4") => Some("Red Hat / Virtio"),
        Some("8086") => Some("Intel"),
        _ => None,
    };
    let identity = match (vendor, product) {
        (Some(vendor), Some(product)) => Some(format!("{vendor}:{product}")),
        (Some(vendor), None) => Some(vendor.to_string()),
        _ => None,
    };
    let details = [driver.map(str::to_string), identity]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let label = match (vendor_name, details.is_empty()) {
        (Some(vendor), true) => vendor.to_string(),
        (Some(vendor), false) => format!("{vendor} ({details})"),
        (None, false) => details,
        (None, true) => return None,
    };
    safe_fact(label)
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
            graphics: Some("Example GPU".into()),
            ..Snapshot::default()
        };

        let report = snapshot.diagnostic_report();

        assert!(report.contains("Ubuntu 26.04 LTS"));
        assert!(report.contains("Example GPU"));
        assert!(!report.contains("private-host"));
        assert!(!report.to_ascii_lowercase().contains("serial"));
        assert!(!report.to_ascii_lowercase().contains("user"));
    }

    #[test]
    fn diagnostics_drop_control_characters_in_public_snapshot_fields() {
        let snapshot = Snapshot {
            operating_system: "Ubuntu\nHostname: private-host".into(),
            kernel: "Linux 6.18".into(),
            architecture: "x86_64".into(),
            processor: Some("CPU\tserial".into()),
            memory: Some("17.2 GB".into()),
            ..Snapshot::default()
        };

        let report = snapshot.diagnostic_report();

        assert!(!report.contains("private-host"));
        assert!(!report.contains("CPU"));
        assert!(report.contains("Memory: 17.2 GB"));
    }

    #[test]
    fn hostname_readback_must_match_the_requested_static_name() {
        let stale = Snapshot {
            hostname: "old-name".into(),
            static_hostname: Some("old-name".into()),
            ..Snapshot::default()
        };

        let error = verify_static_hostname(stale, "new-name").unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Mutation);
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

    #[test]
    fn graphics_labels_use_only_driver_and_public_pci_ids() {
        assert!(is_drm_card_name("card0"));
        assert!(is_drm_card_name("card12"));
        assert!(!is_drm_card_name("card0-DP-1"));
        assert!(!is_drm_card_name("renderD128"));
        assert_eq!(normalize_pci_id("0x10DE".into()).as_deref(), Some("10de"));
        assert_eq!(normalize_pci_id("not-an-id".into()), None);
        assert_eq!(
            graphics_label(Some("nvidia"), Some("10de"), Some("2684")).as_deref(),
            Some("NVIDIA (nvidia, 10de:2684)")
        );
        assert_eq!(
            graphics_label(Some("amdgpu"), None, None).as_deref(),
            Some("amdgpu")
        );
    }

    #[test]
    fn hostname_owner_events_distinguish_loss_and_recovery() {
        assert_eq!(
            owner_change_event("org.freedesktop.hostname1", ""),
            Some(WatchEvent::Unavailable)
        );
        assert_eq!(
            owner_change_event("org.freedesktop.hostname1", ":1.42"),
            Some(WatchEvent::Changed)
        );
        assert_eq!(owner_change_event("org.example.Other", ":1.42"), None);
    }
}
