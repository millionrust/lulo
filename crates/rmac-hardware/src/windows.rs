//! Windows: what the probe (`windows_probe`, Win32 only) reads, and the
//! rules that turn it into a profile. The rules build everywhere, so the
//! tests feed them fake device lists.

use crate::gpu;
use crate::model::{Display, Fingerprint, HardwareProfile};

/// A present Plug and Play device: its setup class name (`Biometric`,
/// `Camera`, `HIDClass`…), instance id, hardware ids and friendly name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub class: String,
    pub instance_id: String,
    pub hardware_ids: Vec<String>,
    pub name: String,
}

/// A network interface from `GetIfTable2`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// IANA ifType: 6 Ethernet, 71 802.11, 243/244 mobile broadband.
    pub if_type: u32,
    /// A real adapter (`HardwareInterface`, not a filter or virtual one).
    pub hardware: bool,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    pub vendor_id: u32,
    pub device_id: u32,
    pub name: String,
    pub dedicated_memory: u64,
    /// DXGI's software adapter (WARP).
    pub software: bool,
}

/// Everything the Windows probe reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub devices: Vec<Device>,
    pub interfaces: Vec<Interface>,
    pub adapters: Vec<Adapter>,
    pub displays: Vec<Display>,
    /// `GetSystemMetrics(SM_DIGITIZER)`.
    pub digitizer: u32,
    /// `GetSystemMetrics(SM_MAXIMUMTOUCHES)`.
    pub max_touches: u32,
    /// `GetSystemMetrics(SM_CONVERTIBLESLATEMODE)`: 0 in tablet (slate) mode.
    pub slate_mode: Option<u32>,
    /// `PowerDeterminePlatformRoleEx`: 2 mobile, 8 slate.
    pub platform_role: u32,
    /// `GetSystemPowerStatus`'s `BatteryFlag`: 128 means no battery.
    pub battery_flag: u8,
    /// `GetAutoRotationState` reported a rotation sensor.
    pub rotation_sensor: bool,
    /// A display driver offers the brightness interface.
    pub brightness_interface: bool,
    /// The Windows Biometric Service is installed.
    pub biometric_service: bool,
    pub memory_bytes: u64,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
}

// SM_DIGITIZER bits (winuser.h).
const NID_INTEGRATED_TOUCH: u32 = 0x01;
const NID_EXTERNAL_TOUCH: u32 = 0x02;
const NID_INTEGRATED_PEN: u32 = 0x04;
const NID_EXTERNAL_PEN: u32 = 0x08;

fn has_id(device: &Device, needle: &str) -> bool {
    device
        .hardware_ids
        .iter()
        .any(|id| id.to_ascii_uppercase().contains(needle))
}

/// A HID collection on USB or Bluetooth (not I²C, PS/2 or ACPI): a mouse
/// someone plugged in or paired, not the touchpad's own mouse collection.
fn external_hid(device: &Device) -> bool {
    let id = device.instance_id.to_ascii_uppercase();
    (id.starts_with("HID\\VID_") && id.contains("&PID_"))
        || id.contains("{00001124-")
        || id.contains("{00001812-")
}

fn class_is(device: &Device, class: &str) -> bool {
    device.class.eq_ignore_ascii_case(class)
}

pub fn profile(facts: &Facts) -> HardwareProfile {
    let mut profile = HardwareProfile {
        memory_bytes: facts.memory_bytes,
        cpu_cores: facts.cpu_cores,
        cpu_threads: facts.cpu_threads,
        displays: facts.displays.clone(),
        ..HardwareProfile::default()
    };
    let lower = |device: &Device| device.name.to_ascii_lowercase();

    let control_method_batteries = facts
        .devices
        .iter()
        .filter(|device| class_is(device, "Battery") && has_id(device, "PNP0C0A"))
        .count() as u32;
    profile.batteries = if facts.battery_flag & 128 != 0 || facts.battery_flag == 255 {
        0
    } else {
        control_method_batteries.max(1)
    };

    profile.touchscreen = facts.digitizer & (NID_INTEGRATED_TOUCH | NID_EXTERNAL_TOUCH) != 0
        || facts
            .devices
            .iter()
            .any(|device| has_id(device, "HID_DEVICE_UP:000D_U:0004"));
    profile.max_touch_points =
        (profile.touchscreen && facts.max_touches > 0).then_some(facts.max_touches);
    profile.pen = facts.digitizer & (NID_INTEGRATED_PEN | NID_EXTERNAL_PEN) != 0
        || facts
            .devices
            .iter()
            .any(|device| has_id(device, "HID_DEVICE_UP:000D_U:0002"));
    profile.precision_touchpad = facts
        .devices
        .iter()
        .any(|device| has_id(device, "HID_DEVICE_UP:000D_U:0005"));
    profile.touchpad = profile.precision_touchpad
        || facts.devices.iter().any(|device| {
            class_is(device, "Mouse")
                && ["touchpad", "touch pad", "synaptics", "alps", "elan"]
                    .iter()
                    .any(|word| lower(device).contains(word))
        });
    profile.pointing_stick = facts.devices.iter().any(|device| {
        let name = lower(device);
        name.contains("trackpoint")
            || name.contains("pointing stick")
            || name.contains("pointstick")
    });
    profile.mouse = facts
        .devices
        .iter()
        .any(|device| class_is(device, "Mouse") && external_hid(device));

    let biometric: Vec<&Device> = facts
        .devices
        .iter()
        .filter(|device| class_is(device, "Biometric"))
        .collect();
    let face = |device: &&Device| {
        let name = lower(device);
        name.contains("face") || name.contains(" ir ")
    };
    profile.fingerprint = Fingerprint {
        present: biometric.iter().any(|device| !face(device)),
        service: facts.biometric_service,
    };
    profile.fingerprint.service &= profile.fingerprint.present;
    let cameras: Vec<&Device> = facts
        .devices
        .iter()
        .filter(|device| class_is(device, "Camera") || class_is(device, "Image"))
        .collect();
    let ir = |device: &&Device| {
        let name = lower(device);
        name.contains(" ir ") || name.ends_with(" ir") || name.contains("infrared")
    };
    profile.ir_camera = cameras.iter().any(ir) || biometric.iter().any(face);
    profile.webcam = cameras.iter().any(|device| !ir(device));

    // Audio endpoints: MMDevice ids start {0.0.0.…} for outputs and
    // {0.0.1.…} for inputs.
    for device in facts
        .devices
        .iter()
        .filter(|device| class_is(device, "AudioEndpoint"))
    {
        let id = device.instance_id.to_ascii_uppercase();
        profile.speakers |= id.contains("{0.0.0.00000000}");
        profile.microphone |= id.contains("{0.0.1.00000000}");
    }

    profile.bluetooth = facts.devices.iter().any(|device| {
        let id = device.instance_id.to_ascii_uppercase();
        class_is(device, "Bluetooth")
            && (id.starts_with("USB\\") || id.starts_with("PCI\\") || id.starts_with("ACPI\\"))
    });
    for interface in facts
        .interfaces
        .iter()
        .filter(|interface| interface.hardware)
    {
        let description = interface.description.to_ascii_lowercase();
        if [
            "bluetooth",
            "virtual",
            "vpn",
            "hyper-v",
            "tap-",
            "wan miniport",
        ]
        .iter()
        .any(|word| description.contains(word))
        {
            continue;
        }
        match interface.if_type {
            71 => profile.wifi = true,
            6 => profile.ethernet = true,
            243 | 244 => profile.cellular = true,
            _ => {}
        }
    }

    let sensor = |device: &Device, usage: &str| has_id(device, usage);
    profile.ambient_light_sensor = facts.devices.iter().any(|device| {
        sensor(device, "HID_DEVICE_UP:0020_U:0041")
            || (class_is(device, "Sensor") && lower(device).contains("light"))
    });
    profile.accelerometer = facts.rotation_sensor
        || facts.devices.iter().any(|device| {
            sensor(device, "HID_DEVICE_UP:0020_U:0073")
                || (class_is(device, "Sensor") && lower(device).contains("accelerometer"))
        });
    // Windows' Dynamic Lighting: a LampArray (HID page 0x59) on the
    // keyboard. Plain backlights have no standard interface.
    profile.keyboard_backlight = facts
        .devices
        .iter()
        .any(|device| sensor(device, "HID_DEVICE_UP:0059_U:0001"));

    profile.portable = profile.batteries > 0 || matches!(facts.platform_role, 2 | 8);
    profile.convertible = facts.platform_role == 8
        || (facts.slate_mode.is_some() && profile.touchscreen && profile.accelerometer);
    profile.tablet_mode = profile.convertible && facts.slate_mode == Some(0);
    profile.backlight = facts.brightness_interface
        || (profile.portable && profile.displays.iter().any(|display| display.internal));

    profile.gpus = facts
        .adapters
        .iter()
        .filter(|adapter| !adapter.software)
        .map(|adapter| {
            gpu::classify(
                adapter.vendor_id,
                adapter.device_id,
                &adapter.name,
                Some(adapter.dedicated_memory),
                false,
            )
        })
        .collect();
    profile
}
