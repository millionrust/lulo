//! GPU vendor, integrated and "old" from PCI ids, shared by both platforms.

use crate::model::{Gpu, GpuVendor};

pub(crate) fn vendor(vendor_id: u32) -> GpuVendor {
    match vendor_id {
        0x8086 => GpuVendor::Intel,
        0x1002 | 0x1022 => GpuVendor::Amd,
        0x10de => GpuVendor::Nvidia,
        0x106b => GpuVendor::Apple,
        0x5143 | 0x4d4f4351 => GpuVendor::Qualcomm,
        // QEMU/Bochs, virtio, VMware, VirtualBox, Hyper-V, Microsoft Basic
        // Render Driver (WARP).
        0x1234 | 0x1af4 | 0x15ad | 0x80ee | 0x1414 => GpuVendor::Virtual,
        _ => GpuVendor::Other,
    }
}

/// Intel's discrete Arc parts (DG1, Alchemist, Battlemage); every other
/// Intel display adapter is in the processor.
fn intel_discrete(device_id: u32) -> bool {
    matches!(device_id, 0x4905..=0x4909 | 0x5690..=0x56ff | 0xe202..=0xe2ff)
}

/// Intel graphics older than Broadwell (2014): Gen 7.5 and earlier,
/// including Bay Trail and the GMA/Ironlake generations.
fn intel_old(device_id: u32) -> bool {
    matches!(
        device_id,
        0x0042 | 0x0046 | 0x0100..=0x0fff | 0x2582..=0x2e9f | 0x3577..=0x35e8 | 0xa001..=0xa011
    )
}

/// One adapter from what the platform says about it. `dedicated_memory` is
/// DXGI's or amdgpu's VRAM size, when known: AMD's APUs carve out at most
/// a little, its graphics cards have their own.
pub(crate) fn classify(
    vendor_id: u32,
    device_id: u32,
    name: &str,
    dedicated_memory: Option<u64>,
    legacy_amd_driver: bool,
) -> Gpu {
    let vendor = vendor(vendor_id);
    let integrated = match vendor {
        GpuVendor::Intel => !intel_discrete(device_id),
        GpuVendor::Amd => dedicated_memory.is_some_and(|bytes| bytes <= 1 << 30),
        GpuVendor::Apple | GpuVendor::Qualcomm => true,
        GpuVendor::Nvidia | GpuVendor::Virtual | GpuVendor::Other => false,
    };
    let old = match vendor {
        GpuVendor::Intel => intel_old(device_id),
        // `radeon` drives the generations before GCN's later parts; the
        // APUs among them (Kabini, Kaveri) are slow for blur too.
        GpuVendor::Amd => legacy_amd_driver,
        _ => false,
    };
    Gpu {
        vendor,
        vendor_id,
        device_id,
        name: name.to_string(),
        integrated,
        old,
    }
}
