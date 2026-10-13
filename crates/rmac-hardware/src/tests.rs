use std::fs;
use std::path::{Path, PathBuf};

use crate::linux::{self, Bits};
use crate::windows::{self as win, Adapter, Device, Facts, Interface};
use crate::{Display, GpuVendor, HardwareProfile, SpecClass};

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rmac-hardware-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, contents: &str) -> &Self {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        self
    }

    #[cfg(unix)]
    fn bytes(&self, relative: &str, contents: &[u8]) -> &Self {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        self
    }

    #[cfg(unix)]
    /// An input event device with the given sysfs capability bitmaps.
    fn input(
        &self,
        event: &str,
        name: &str,
        bus: &str,
        caps: &[(&str, &str)],
        props: &str,
    ) -> &Self {
        let device = format!("sys/class/input/{event}/device");
        self.write(&format!("{device}/name"), name);
        self.write(&format!("{device}/id/bustype"), bus);
        self.write(&format!("{device}/properties"), props);
        for kind in ["ev", "key", "abs", "rel", "sw"] {
            let value = caps
                .iter()
                .find(|(candidate, _)| *candidate == kind)
                .map_or("0", |(_, value)| value);
            self.write(&format!("{device}/capabilities/{kind}"), value);
        }
        self
    }

    #[cfg(unix)]
    fn roots(&self) -> linux::Roots {
        linux::Roots {
            sys: self.0.join("sys"),
            proc_dir: self.0.join("proc"),
            root: self.0.join("root"),
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An EDID with one detailed timing: 1366x768, 76.3 MHz pixel clock,
/// 1560x806 total (60.68 Hz), 309x174 mm.
fn edid() -> Vec<u8> {
    let mut edid = vec![0u8; 128];
    edid[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    let d = &mut edid[54..72];
    d[0..2].copy_from_slice(&7630u16.to_le_bytes());
    let (h_active, h_blank, v_active, v_blank) = (1366u32, 194u32, 768u32, 38u32);
    d[2] = (h_active & 0xff) as u8;
    d[3] = (h_blank & 0xff) as u8;
    d[4] = (((h_active >> 8) << 4) | (h_blank >> 8)) as u8;
    d[5] = (v_active & 0xff) as u8;
    d[6] = (v_blank & 0xff) as u8;
    d[7] = (((v_active >> 8) << 4) | (v_blank >> 8)) as u8;
    d[12] = (309 & 0xff) as u8;
    d[13] = 174;
    d[14] = ((309 >> 8) << 4) as u8;
    edid
}

/// The reference laptop, an EliteBook 840 G2, as its sysfs reads.
#[cfg(unix)]
fn elitebook(tree: &Tree) {
    tree.write("sys/class/power_supply/AC/type", "Mains")
        .write("sys/class/power_supply/BAT0/type", "Battery")
        .write("sys/class/power_supply/BAT0/scope", "System")
        // A Bluetooth mouse's battery is not the PC's.
        .write("sys/class/power_supply/hid-mouse-battery/type", "Battery")
        .write("sys/class/power_supply/hid-mouse-battery/scope", "Device")
        .write("sys/class/backlight/intel_backlight/brightness", "120")
        .write("sys/class/leds/hp::hddprotect/brightness", "0")
        .write("sys/class/leds/input3::capslock/brightness", "0")
        .write("sys/class/bluetooth/hci0/address", "00:00:00:00:00:00")
        .write("sys/class/bluetooth/hci0:256/type", "ACL")
        .write("sys/class/net/lo/type", "772")
        .write("sys/class/net/enp0s25/type", "1")
        .write("sys/class/net/enp0s25/device/vendor", "0x8086")
        .write("sys/class/net/wlo1/type", "1")
        .write("sys/class/net/wlo1/uevent", "DEVTYPE=wlan\nINTERFACE=wlo1")
        .write("sys/class/net/wlo1/device/vendor", "0x8086")
        .write("sys/class/net/docker0/type", "1")
        .write("sys/class/net/docker0/uevent", "DEVTYPE=bridge")
        .write(
            "sys/class/video4linux/video0/name",
            "HP HD Webcam: HP HD Webcam",
        )
        .write("sys/class/video4linux/video0/index", "0")
        .write(
            "sys/class/video4linux/video1/name",
            "HP HD Webcam: HP HD Webcam",
        )
        .write("sys/class/video4linux/video1/index", "1")
        .write("sys/class/video4linux/v4l-touch0/name", "Synaptics RMI4")
        .write("sys/class/sound/pcmC0D0c/dev", "116:2")
        .write("sys/class/sound/pcmC0D0p/dev", "116:3")
        .write("sys/bus/usb/devices/1-1.1/idVendor", "8087")
        .write("sys/bus/usb/devices/1-1.1/idProduct", "0a2a")
        .write("sys/bus/usb/devices/1-1.3/idVendor", "138a")
        .write("sys/bus/usb/devices/1-1.3/idProduct", "003f")
        .write("sys/bus/usb/devices/1-1.4/idVendor", "03eb")
        .write("sys/bus/usb/devices/1-1.4/idProduct", "8a47")
        .write(
            "sys/bus/usb/devices/1-1.4/product",
            "Atmel maXTouch Digitizer",
        )
        .write("root/usr/libexec/fprintd", "")
        .write("sys/class/dmi/id/chassis_type", "10")
        .write("sys/class/drm/card1/device/vendor", "0x8086")
        .write("sys/class/drm/card1/device/device", "0x1616")
        .write("sys/class/drm/card1-eDP-1/status", "connected")
        .write("sys/class/drm/card1-eDP-1/modes", "1366x768\n1024x768")
        .bytes("sys/class/drm/card1-eDP-1/edid", &edid())
        .write("sys/class/drm/card1-HDMI-A-1/status", "disconnected")
        .write(
            "proc/meminfo",
            "MemTotal:        6991324 kB\nMemFree: 1 kB\n",
        );
    for cpu in 0..4 {
        tree.write(
            &format!("sys/devices/system/cpu/cpu{cpu}/topology/core_id"),
            &(cpu / 2).to_string(),
        )
        .write(
            &format!("sys/devices/system/cpu/cpu{cpu}/topology/physical_package_id"),
            "0",
        );
    }
    tree.input(
        "event1",
        "Lid Switch",
        "0019",
        &[("ev", "21"), ("sw", "1")],
        "0",
    )
    // Exactly the 840 G2's bitmaps.
    .input(
        "event4",
        "Atmel Atmel maXTouch Digitizer",
        "0003",
        &[
            ("ev", "1b"),
            ("key", "400 0 0 0 0 0"),
            ("abs", "260800000000003"),
        ],
        "2",
    )
    .input(
        "event15",
        "Synaptics TM2768-002",
        "001d",
        &[
            ("ev", "b"),
            ("key", "e520 30000 0 0 0 0"),
            ("abs", "6f3800001000003"),
        ],
        "1",
    )
    .input(
        "event5",
        "PS/2 Generic Mouse",
        "0011",
        &[("ev", "7"), ("key", "70000 0 0 0 0"), ("rel", "3")],
        "1",
    )
    .input(
        "event8",
        "ST LIS3LV02DL Accelerometer",
        "0019",
        &[("ev", "9"), ("abs", "7")],
        "40",
    );
}

#[test]
fn capability_bitmaps_read_most_significant_word_first() {
    let bits = Bits::parse("400 0 0 0 0 0");
    assert!(bits.has(0x14a));
    assert!(!bits.has(0x145));
    let bits = Bits::parse("e520 30000 0 0 0 0");
    assert!(bits.has(0x145) && bits.has(0x14d) && bits.has(0x110));
    assert!(!Bits::parse("").has(0));
}

#[test]
fn edid_gives_refresh_and_size() {
    let (refresh, size) = linux::edid_timing(&edid());
    assert_eq!(refresh, Some(60_682));
    assert_eq!(size, Some((309, 174)));
    assert_eq!(linux::edid_timing(&[0; 10]), (None, None));
}

// sysfs names (`hci0:256`, `hp::hddprotect`) are not Windows file names.
#[cfg(unix)]
#[test]
fn the_elitebook_is_detected_from_sysfs() {
    let tree = Tree::new("elitebook");
    elitebook(&tree);
    let profile = linux::detect(&tree.roots()).classify();
    assert_eq!(profile.batteries, 1);
    assert!(profile.backlight && !profile.keyboard_backlight);
    assert!(profile.touchscreen);
    assert!(!profile.pen);
    assert!(profile.touchpad && profile.precision_touchpad);
    assert!(
        profile.pointing_stick,
        "the PS/2 mouse beside a touchpad is the stick"
    );
    assert!(!profile.mouse);
    assert!(profile.fingerprint.present && profile.fingerprint.service);
    assert!(profile.webcam && !profile.ir_camera);
    assert!(profile.microphone && profile.speakers);
    assert!(profile.bluetooth && profile.wifi && profile.ethernet && !profile.cellular);
    assert!(
        !profile.accelerometer,
        "the disk-protection sensor cannot rotate"
    );
    assert!(!profile.convertible && profile.portable);
    assert_eq!(profile.displays.len(), 1);
    let panel = &profile.displays[0];
    assert!(panel.internal);
    assert_eq!((panel.width, panel.height), (1366, 768));
    assert_eq!(panel.refresh_millihertz, Some(60_682));
    assert_eq!(panel.ppi, Some(112));
    assert!(!panel.hidpi());
    assert_eq!(profile.gpus.len(), 1);
    assert_eq!(profile.gpus[0].vendor, GpuVendor::Intel);
    assert!(profile.gpus[0].integrated && !profile.gpus[0].old);
    assert_eq!((profile.cpu_cores, profile.cpu_threads), (2, 4));
    assert_eq!(profile.memory_bytes, 6_991_324 * 1024);
    assert_eq!(profile.spec, SpecClass::Low);
    assert_eq!(profile.spec_reasons, vec!["4 CPU threads".to_string()]);
    let adapt = profile.adaptations();
    assert!(adapt.battery && adapt.wifi && adapt.bluetooth && adapt.display_brightness);
    assert!(!adapt.keyboard_brightness);
    assert!(adapt.touch_targets && adapt.fingerprint);
    assert!(adapt.reduce_transparency && adapt.reduce_motion);
}

// sysfs names (`hci0:256`, `hp::hddprotect`) are not Windows file names.
#[cfg(unix)]
#[test]
fn a_desktop_without_radios_shows_none_of_them() {
    let tree = Tree::new("desktop");
    tree.write("sys/class/net/eno1/type", "1")
        .write("sys/class/net/eno1/device/vendor", "0x10ec")
        .write("sys/class/drm/card0/device/vendor", "0x10de")
        .write("sys/class/drm/card0/device/device", "0x2484")
        .write("sys/class/drm/card0-DP-1/status", "connected")
        .write("sys/class/drm/card0-DP-1/modes", "3840x2160")
        .write("sys/class/dmi/id/chassis_type", "3")
        .write("proc/meminfo", "MemTotal: 16303448 kB\n")
        .input(
            "event3",
            "Logitech USB Receiver",
            "0003",
            &[("ev", "17"), ("key", "ffff0000 0 0 0 0"), ("rel", "1943")],
            "0",
        );
    for cpu in 0..12 {
        tree.write(
            &format!("sys/devices/system/cpu/cpu{cpu}/topology/core_id"),
            &(cpu / 2).to_string(),
        );
    }
    let profile = linux::detect(&tree.roots()).classify();
    assert_eq!(profile.batteries, 0);
    assert!(!profile.wifi && !profile.bluetooth && !profile.backlight);
    assert!(profile.ethernet && profile.mouse && !profile.touchpad);
    assert!(!profile.portable);
    assert!(!profile.displays[0].internal);
    assert!(!profile.gpus[0].integrated);
    assert_eq!((profile.cpu_cores, profile.cpu_threads), (6, 12));
    assert_eq!(profile.spec, SpecClass::Standard);
    let adapt = profile.adaptations();
    assert!(!adapt.battery && !adapt.wifi && !adapt.bluetooth && !adapt.display_brightness);
    assert!(!adapt.nearby_sharing && !adapt.reduce_motion);
}

// sysfs names (`hci0:256`, `hp::hddprotect`) are not Windows file names.
#[cfg(unix)]
#[test]
fn a_convertible_with_pen_and_sensors() {
    let tree = Tree::new("convertible");
    tree.write("sys/class/leds/tpacpi::kbd_backlight/brightness", "1")
        .write("sys/class/net/wwan0/type", "519")
        .write("sys/class/net/wwan0/uevent", "DEVTYPE=wwan")
        .write("sys/bus/iio/devices/iio:device0/name", "accel_3d")
        .write("sys/bus/iio/devices/iio:device0/in_accel_x_raw", "0")
        .write("sys/bus/iio/devices/iio:device0/in_accel_y_raw", "0")
        .write("sys/bus/iio/devices/iio:device0/in_accel_z_raw", "0")
        .write("sys/bus/iio/devices/iio:device1/name", "als")
        .write("sys/bus/iio/devices/iio:device1/in_illuminance_raw", "10")
        .write("sys/class/video4linux/video2/name", "Integrated IR Camera")
        .write("sys/class/video4linux/video2/index", "0")
        .write("sys/bus/usb/devices/3-4/idVendor", "06cb")
        .write("sys/bus/usb/devices/3-4/idProduct", "00bd")
        .input(
            "event2",
            "Intel HID switches",
            "0019",
            &[("ev", "21"), ("sw", "2")],
            "0",
        )
        .input(
            "event9",
            "Wacom Pen and multitouch sensor Pen",
            "0003",
            &[("ev", "b"), ("key", "c01 0 0 0 0 0"), ("abs", "3")],
            "2",
        )
        // 1 GB of memory, old Haswell graphics.
        .write("sys/class/drm/card0/device/vendor", "0x8086")
        .write("sys/class/drm/card0/device/device", "0x0a16")
        .write("proc/meminfo", "MemTotal: 3900000 kB\n");
    let profile = linux::detect(&tree.roots()).classify();
    assert!(profile.keyboard_backlight && profile.cellular);
    assert!(profile.accelerometer && profile.ambient_light_sensor);
    assert!(profile.ir_camera && !profile.webcam);
    assert!(profile.fingerprint.present && !profile.fingerprint.service);
    assert!(profile.convertible && profile.pen && !profile.touchscreen);
    assert!(profile.gpus[0].old);
    assert_eq!(profile.spec, SpecClass::Low);
    assert_eq!(profile.spec_reasons.len(), 2, "{:?}", profile.spec_reasons);
    assert!(
        !profile.adaptations().fingerprint,
        "no fprintd: do not offer it"
    );
}

// sysfs names (`hci0:256`, `hp::hddprotect`) are not Windows file names.
#[cfg(unix)]
#[test]
fn an_empty_tree_is_an_empty_profile() {
    let tree = Tree::new("empty");
    let profile = linux::detect(&tree.roots()).classify();
    assert_eq!(profile.spec, SpecClass::Standard);
    assert!(profile
        .summary()
        .starts_with("Lulo hardware: standard spec;"));
}

fn device(class: &str, instance: &str, ids: &[&str], name: &str) -> Device {
    Device {
        class: class.into(),
        instance_id: instance.into(),
        hardware_ids: ids.iter().map(|id| id.to_string()).collect(),
        name: name.into(),
    }
}

/// Ruban's HP 245 G10, as its device manager lists it (2026-10-13).
fn hp_245() -> Facts {
    Facts {
        devices: vec![
            device(
                "AudioEndpoint",
                "SWD\\MMDEVAPI\\{0.0.1.00000000}.{158A}",
                &[],
                "",
            ),
            device(
                "AudioEndpoint",
                "SWD\\MMDEVAPI\\{0.0.0.00000000}.{9D6F}",
                &[],
                "",
            ),
            device(
                "Battery",
                "ACPI\\ACPI0003\\2&DABA3FF&0",
                &["ACPI\\ACPI0003"],
                "Microsoft AC Adapter",
            ),
            device(
                "Battery",
                "ACPI\\PNP0C0A\\1",
                &["ACPI\\PNP0C0A"],
                "Microsoft ACPI-Compliant Control Method Battery",
            ),
            device(
                "Bluetooth",
                "USB\\VID_0BDA&PID_B85D\\00E04C000001",
                &[],
                "Realtek Wireless Bluetooth Adapter",
            ),
            device("Bluetooth", "BTHENUM\\{0000110E}", &[], "Avrcp Transport"),
            device(
                "Camera",
                "USB\\VID_0408&PID_548F&MI_00\\7&1AB5",
                &[],
                "HP True Vision HD Camera",
            ),
            device(
                "HIDClass",
                "HID\\ELAN07B8&COL03\\4&384B7595&0&0002",
                &[
                    "HID\\VID_04F3&UP:000D_U:0005",
                    "HID_DEVICE_UP:000D_U:0005",
                    "HID_DEVICE",
                ],
                "ELAN PrecisionTouchpad Filter Driver",
            ),
            device(
                "Mouse",
                "HID\\ELAN07B8&COL01\\4&384B7595&0&0000",
                &["HID\\VID_04F3&UP:0001_U:0002", "HID_DEVICE_SYSTEM_MOUSE"],
                "ELAN Input Device",
            ),
        ],
        interfaces: vec![
            Interface {
                if_type: 71,
                hardware: true,
                description: "Realtek RTL8852BE-VS WiFi 6 802.11ax PCIe Adapter".into(),
            },
            Interface {
                if_type: 71,
                hardware: false,
                description: "Microsoft Wi-Fi Direct Virtual Adapter".into(),
            },
            Interface {
                if_type: 6,
                hardware: false,
                description: "WAN Miniport (IP)".into(),
            },
            Interface {
                if_type: 6,
                hardware: true,
                description: "Bluetooth Device (Personal Area Network)".into(),
            },
        ],
        adapters: vec![
            Adapter {
                vendor_id: 0x1002,
                device_id: 0x1506,
                name: "AMD Radeon(TM) Graphics".into(),
                dedicated_memory: 512 << 20,
                software: false,
            },
            Adapter {
                vendor_id: 0x1414,
                device_id: 0x8c,
                name: "Microsoft Basic Render Driver".into(),
                dedicated_memory: 0,
                software: true,
            },
        ],
        displays: vec![Display {
            name: "DISPLAY1".into(),
            internal: true,
            width: 1366,
            height: 768,
            refresh_millihertz: Some(60_000),
            scale: Some(1.0),
            ppi: None,
        }],
        digitizer: 0,
        max_touches: 0,
        slate_mode: Some(1),
        platform_role: 2,
        battery_flag: 1,
        rotation_sensor: false,
        brightness_interface: true,
        biometric_service: true,
        memory_bytes: 7_772_880_896,
        cpu_cores: 4,
        cpu_threads: 8,
    }
}

#[test]
fn ruban_s_laptop_from_its_device_list() {
    let profile = win::profile(&hp_245()).classify();
    assert_eq!(profile.batteries, 1);
    assert!(profile.backlight);
    assert!(!profile.touchscreen && profile.max_touch_points.is_none() && !profile.pen);
    assert!(profile.touchpad && profile.precision_touchpad);
    assert!(
        !profile.mouse,
        "the touchpad's own mouse collection is on I2C"
    );
    assert!(!profile.pointing_stick);
    assert!(!profile.fingerprint.present && !profile.ir_camera);
    assert!(profile.webcam && profile.microphone && profile.speakers);
    assert!(profile.bluetooth && profile.wifi);
    assert!(
        !profile.ethernet,
        "virtual and Bluetooth adapters are not Ethernet"
    );
    assert!(profile.portable && !profile.convertible && !profile.tablet_mode);
    assert_eq!(profile.gpus.len(), 1);
    assert!(profile.gpus[0].integrated && profile.gpus[0].vendor == GpuVendor::Amd);
    assert_eq!(profile.spec, SpecClass::Standard);
}

#[test]
fn a_windows_tablet_with_touch_pen_hello_and_a_usb_mouse() {
    let mut facts = Facts {
        digitizer: 0x01 | 0x04 | 0x40 | 0x80,
        max_touches: 10,
        slate_mode: Some(0),
        platform_role: 8,
        battery_flag: 8,
        rotation_sensor: true,
        biometric_service: true,
        memory_bytes: 4 << 30,
        cpu_cores: 2,
        cpu_threads: 4,
        ..Facts::default()
    };
    facts.devices = vec![
        device(
            "Biometric",
            "USB\\VID_27C6&PID_538C\\1",
            &[],
            "Goodix MOC Fingerprint",
        ),
        device(
            "Biometric",
            "SWD\\FACE\\1",
            &[],
            "Windows Hello Face Software Device",
        ),
        device(
            "Camera",
            "USB\\VID_0BDA&PID_5830&MI_02\\1",
            &[],
            "Integrated IR Camera",
        ),
        device(
            "Camera",
            "USB\\VID_0BDA&PID_5830&MI_00\\1",
            &[],
            "Integrated Camera",
        ),
        device(
            "Mouse",
            "HID\\VID_046D&PID_C52B&MI_01&COL01\\8&1",
            &["HID\\VID_046D&PID_C52B&MI_01&Col01"],
            "HID-compliant mouse",
        ),
        device(
            "Sensor",
            "HID\\SENSOR\\1",
            &["HID_DEVICE_UP:0020_U:0041"],
            "Light sensor",
        ),
        device(
            "HIDClass",
            "HID\\KBD\\1",
            &["HID_DEVICE_UP:0059_U:0001"],
            "LampArray",
        ),
    ];
    facts.interfaces = vec![Interface {
        if_type: 243,
        hardware: true,
        description: "Snapdragon X12 LTE".into(),
    }];
    let profile = win::profile(&facts).classify();
    assert!(profile.touchscreen && profile.max_touch_points == Some(10) && profile.pen);
    assert!(profile.fingerprint.present && profile.fingerprint.service);
    assert!(profile.ir_camera && profile.webcam && profile.mouse);
    assert!(profile.ambient_light_sensor && profile.accelerometer && profile.keyboard_backlight);
    assert!(profile.cellular && !profile.wifi);
    assert!(profile.convertible && profile.tablet_mode);
    assert_eq!(profile.batteries, 1);
    assert_eq!(profile.spec, SpecClass::Low);
    assert_eq!(profile.spec_reasons.len(), 2);
}

#[test]
fn no_system_battery_flag_means_no_battery() {
    let mut facts = hp_245();
    facts.battery_flag = 128;
    facts.platform_role = 1;
    let profile = win::profile(&facts);
    assert_eq!(profile.batteries, 0);
    assert!(!profile.adaptations().battery);
    assert!(!profile.portable);
}

#[test]
fn uevents_that_matter() {
    let message = |action: &str, subsystem: &str| {
        format!("{action}@/devices/x\0ACTION={action}\0SUBSYSTEM={subsystem}\0").into_bytes()
    };
    assert!(crate::watch::relevant_uevent(&message("add", "input")));
    assert!(crate::watch::relevant_uevent(&message("remove", "usb")));
    assert!(crate::watch::relevant_uevent(&message("change", "drm")));
    assert!(!crate::watch::relevant_uevent(&message(
        "change",
        "power_supply"
    )));
    assert!(!crate::watch::relevant_uevent(&message("add", "block")));
    assert!(!crate::watch::relevant_uevent(b"libudev\0\0\0"));
}

#[test]
fn the_store_records_the_first_run_and_changes_only() {
    let tree = Tree::new("store");
    let path: &Path = &tree.0.join("config/rmac/hardware.json");
    let mut profile = win::profile(&hp_245()).classify();
    profile.detection_micros = 4000;
    assert_eq!(
        crate::store::record_at(path, &profile),
        crate::store::Recorded::FirstRun
    );
    assert_eq!(crate::store::load_from(path).as_ref(), Some(&profile));
    profile.detection_micros = 9000;
    assert_eq!(
        crate::store::record_at(path, &profile),
        crate::store::Recorded::Unchanged
    );
    profile.mouse = true;
    assert_eq!(
        crate::store::record_at(path, &profile),
        crate::store::Recorded::Changed
    );
}

#[test]
fn before_detection_everything_shows() {
    let adapt = crate::Adaptations::default();
    assert!(adapt.battery && adapt.wifi && adapt.bluetooth && adapt.display_brightness);
    assert!(!adapt.reduce_motion && !adapt.reduce_transparency && !adapt.touch_targets);
}

#[test]
fn an_unknown_profile_parses_from_older_json() {
    let profile: HardwareProfile = serde_json::from_str(r#"{"batteries":2}"#).unwrap();
    assert_eq!(profile.batteries, 2);
    assert_eq!(profile.spec, SpecClass::Standard);
}

#[test]
fn detection_on_this_machine_is_within_budget_and_does_not_panic() {
    let profile = crate::detect();
    // CI machines vary; the budget itself is checked by `rmac-hardware
    // --check`. Here detection must simply finish and classify.
    assert!(profile.detection_micros < 10 * crate::BUDGET_MICROS);
}
