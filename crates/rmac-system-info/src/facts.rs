#[cfg(not(target_os = "macos"))]
use std::fs;
use std::io::Read;
#[cfg(not(target_os = "macos"))]
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use crate::api::valid_fact;

const MAX_COMMAND_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(not(target_os = "macos"))]
const MAX_GRAPHICS_DEVICES: usize = 16;
#[cfg(not(target_os = "macos"))]
const MAX_DRM_ENTRIES: usize = 128;

pub(crate) fn command(program: &str, args: &[&str]) -> Option<String> {
    command_text(program, args).and_then(safe_fact)
}

pub(crate) fn command_text(program: &str, args: &[&str]) -> Option<String> {
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
pub(crate) fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    read_bounded(path, 64 * 1024).and_then(safe_fact)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn environment_value(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(safe_fact)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn read_os_release(key: &str) -> Option<String> {
    read_bounded("/etc/os-release", 64 * 1024).and_then(|contents| os_release_value(&contents, key))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn read_bounded(path: impl AsRef<Path>, limit: u64) -> Option<String> {
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

pub(crate) fn safe_fact(value: String) -> Option<String> {
    let value = value.trim();
    valid_fact(value).then(|| value.to_string())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn os_release_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key)
            .then(|| value.trim().trim_matches(['\'', '"']).to_string())
            .and_then(safe_fact)
    })
}

pub(crate) fn colon_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        (candidate.trim() == key)
            .then(|| value.trim().to_string())
            .and_then(safe_fact)
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn processor_from_cpuinfo(contents: &str) -> Option<String> {
    ["model name", "Hardware", "Processor"]
        .into_iter()
        .find_map(|key| colon_value(contents, key))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn memory_from_meminfo(contents: &str) -> Option<String> {
    colon_value(contents, "MemTotal")
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kibibytes| format_memory_bytes(kibibytes.saturating_mul(1024)))
}

pub(crate) fn format_memory_bytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

pub(crate) fn architecture_label(architecture: &str) -> &str {
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
pub(crate) fn linux_graphics() -> Option<String> {
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
pub(crate) fn linux_graphics_device(card: &Path) -> Option<String> {
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
pub(crate) fn is_drm_card_name(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn normalize_pci_id(value: String) -> Option<String> {
    let value = value.strip_prefix("0x").unwrap_or(&value);
    (value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn graphics_label(
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
