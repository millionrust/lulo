//! rmac: load only the Vulkan drivers for the GPUs this machine has.
//!
//! The Vulkan loader opens every installed driver manifest (Ubuntu ships
//! Intel, Intel-hasvk, Radeon, Nouveau, Asahi, VirtIO, gfxstream and
//! lavapipe) when an instance is created, and each driver initialises
//! before the loader can ask it about devices. On the reference laptop that
//! cost ~23 ms of every process's first window (docs/perf/
//! speed-round-1-2026-10-06.md). Before the first instance exists, this
//! points `VK_DRIVER_FILES` at just the manifests whose name matches the
//! vendor of a DRM render node that is present. It changes nothing when the
//! user or session already chose drivers, when a render node's vendor is not
//! one it knows, or when no manifest matches, so an unusual machine keeps
//! the loader's full search (and `WgpuRenderer::new` keeps its GL fallback).

#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
const DRIVER_ENV: [&str; 3] = ["VK_DRIVER_FILES", "VK_ICD_FILENAMES", "VK_ADD_DRIVER_FILES"];

/// Manifest file-name fragments for a PCI vendor id, or `None` for a vendor
/// this does not know (the caller then leaves the loader alone).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn vendor_manifest_names(vendor: u32) -> Option<&'static [&'static str]> {
    Some(match vendor {
        0x8086 => &["intel"],
        0x1002 => &["radeon", "amd"],
        0x10de => &["nvidia", "nouveau"],
        0x1af4 => &["virtio"],
        _ => return None,
    })
}

/// The manifests to load for the vendors present, or `None` to leave the
/// loader's own search untouched.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn select_manifests(
    vendors: &[u32],
    manifests: &[std::path::PathBuf],
) -> Option<Vec<std::path::PathBuf>> {
    if vendors.is_empty() {
        return None;
    }
    let mut fragments = Vec::new();
    for vendor in vendors {
        fragments.extend_from_slice(vendor_manifest_names(*vendor)?);
    }
    let selected: Vec<_> = manifests
        .iter()
        .filter(|manifest| {
            let name = manifest
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            name.ends_with(".json") && fragments.iter().any(|fragment| name.contains(fragment))
        })
        .cloned()
        .collect();
    (!selected.is_empty()).then_some(selected)
}

#[cfg(target_os = "linux")]
fn render_node_vendors() -> Vec<u32> {
    let mut vendors = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return vendors;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("renderD") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path().join("device/vendor")) else {
            // A render node without a PCI vendor (an SoC GPU): unknown.
            return vec![0];
        };
        let Ok(vendor) = u32::from_str_radix(text.trim().trim_start_matches("0x"), 16) else {
            return vec![0];
        };
        if !vendors.contains(&vendor) {
            vendors.push(vendor);
        }
    }
    vendors
}

#[cfg(target_os = "linux")]
fn manifest_dirs() -> Vec<PathBuf> {
    let config = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    let data = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs: Vec<PathBuf> = config
        .split(':')
        .map(|dir| Path::new(dir).join("vulkan/icd.d"))
        .collect();
    dirs.push(PathBuf::from("/etc/vulkan/icd.d"));
    dirs.extend(
        data.split(':')
            .map(|dir| Path::new(dir).join("vulkan/icd.d")),
    );
    dirs
}

/// Restrict the Vulkan loader to the present GPUs' drivers. Call once, early,
/// on the main thread before any Vulkan instance or other threads that read
/// the environment exist.
pub fn restrict_vulkan_drivers_to_present_gpus() {
    #[cfg(target_os = "linux")]
    {
        if DRIVER_ENV
            .iter()
            .any(|name| std::env::var_os(name).is_some())
        {
            return;
        }
        let mut manifests = Vec::new();
        for dir in manifest_dirs() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                manifests.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
        let Some(selected) = select_manifests(&render_node_vendors(), &manifests) else {
            return;
        };
        let value = selected
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":");
        log::info!("Vulkan drivers limited to the present GPUs: {value}");
        // SAFETY: called from the platform's constructor on the main thread,
        // before GPUI starts its executor threads or loads any C library that
        // reads the environment concurrently.
        unsafe { std::env::set_var("VK_DRIVER_FILES", value) };
        RESTRICTED.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(target_os = "linux")]
static RESTRICTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Drop the restriction once this process's Vulkan instance exists (the
/// loader has read it by then), so programs this process starts -- a
/// Terminal's shell, an app the Dock opens -- see the normal driver search.
pub(crate) fn release_vulkan_driver_restriction() {
    #[cfg(target_os = "linux")]
    if RESTRICTED.swap(false, std::sync::atomic::Ordering::AcqRel) {
        // SAFETY: Rust's own environment access is serialised by std; this
        // runs once, on the UI thread, right after the instance is created.
        unsafe { std::env::remove_var("VK_DRIVER_FILES") };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn manifests() -> Vec<PathBuf> {
        [
            "intel_icd.json",
            "intel_hasvk_icd.json",
            "radeon_icd.json",
            "nouveau_icd.json",
            "lvp_icd.json",
            "asahi_icd.json",
        ]
        .iter()
        .map(|name| PathBuf::from("/usr/share/vulkan/icd.d").join(name))
        .collect()
    }

    #[test]
    fn keeps_only_the_present_vendors_drivers() {
        let selected = select_manifests(&[0x8086], &manifests()).unwrap();
        let names: Vec<_> = selected
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["intel_icd.json", "intel_hasvk_icd.json"]);
    }

    #[test]
    fn hybrid_machines_keep_both_vendors() {
        let selected = select_manifests(&[0x8086, 0x10de], &manifests()).unwrap();
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn unknown_or_missing_gpus_leave_the_loader_alone() {
        assert!(select_manifests(&[], &manifests()).is_none());
        assert!(select_manifests(&[0x15ad], &manifests()).is_none());
        assert!(select_manifests(&[0x8086, 0], &manifests()).is_none());
        assert!(select_manifests(&[0x1af4], &manifests()).is_none());
    }
}
