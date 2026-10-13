//! Linux detection from sysfs, /proc and a few files under /usr. Only small
//! files are read and only once each: no device nodes, no D-Bus, no
//! subprocesses, so a full read takes a few milliseconds.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::gpu;
use crate::model::{Display, Fingerprint, HardwareProfile};

const MAX_ENTRIES: usize = 512;
const MAX_FILE: usize = 64 * 1024;

/// Where to read from: `/` on a real PC, a scratch tree in tests and in the
/// nested behaviour session (`LULO_FAKE_SYS_ROOT` names its `sys`).
#[derive(Clone, Debug)]
pub struct Roots {
    pub sys: PathBuf,
    pub proc_dir: PathBuf,
    /// The filesystem root for the fprintd check.
    pub root: PathBuf,
}

impl Roots {
    pub fn system() -> Self {
        match std::env::var_os("LULO_FAKE_SYS_ROOT").map(PathBuf::from) {
            Some(sys) if sys.is_absolute() => Self {
                root: sys.join("fake-root"),
                proc_dir: sys.join("fake-proc"),
                sys,
            },
            _ => Self {
                sys: PathBuf::from("/sys"),
                proc_dir: PathBuf::from("/proc"),
                root: PathBuf::from("/"),
            },
        }
    }
}

fn entries(path: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = fs::read_dir(path)
        .map(|items| {
            items
                .flatten()
                .take(MAX_ENTRIES)
                .map(|item| item.path())
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    list
}

fn read(path: &Path) -> String {
    fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() <= MAX_FILE)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn hex(value: &str) -> Option<u32> {
    u32::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()
}

/// A sysfs capability bitmap: hex words, most significant first, each a
/// kernel `long` wide.
pub(crate) struct Bits(Vec<u64>);

impl Bits {
    pub(crate) fn parse(text: &str) -> Self {
        let words: Vec<u64> = text
            .split_whitespace()
            .rev()
            .map(|word| u64::from_str_radix(word, 16).unwrap_or(0))
            .collect();
        // On 32-bit kernels two words make one u64; fold them.
        if usize::BITS == 32 {
            let folded = words
                .chunks(2)
                .map(|pair| pair[0] | pair.get(1).map_or(0, |high| high << 32))
                .collect();
            return Self(folded);
        }
        Self(words)
    }

    pub(crate) fn has(&self, bit: u32) -> bool {
        self.0
            .get((bit / 64) as usize)
            .is_some_and(|word| word & (1 << (bit % 64)) != 0)
    }
}

// linux/input-event-codes.h
const EV_KEY: u32 = 0x01;
const EV_REL: u32 = 0x02;
const EV_ABS: u32 = 0x03;
const EV_SW: u32 = 0x05;
const BTN_MOUSE: u32 = 0x110;
const BTN_TOOL_PEN: u32 = 0x140;
const BTN_TOOL_FINGER: u32 = 0x145;
const BTN_TOOL_DOUBLETAP: u32 = 0x14d;
const BTN_TOUCH: u32 = 0x14a;
const BTN_STYLUS: u32 = 0x14b;
const REL_X: u32 = 0x00;
const REL_Y: u32 = 0x01;
const ABS_X: u32 = 0x00;
const ABS_Y: u32 = 0x01;
const ABS_MT_POSITION_X: u32 = 0x35;
const SW_LID: u32 = 0x00;
const SW_TABLET_MODE: u32 = 0x01;
const INPUT_PROP_DIRECT: u32 = 0x01;
const INPUT_PROP_POINTING_STICK: u32 = 0x05;
const INPUT_PROP_ACCELEROMETER: u32 = 0x06;
const BUS_USB: u32 = 0x03;
const BUS_BLUETOOTH: u32 = 0x05;

#[derive(Default)]
struct Inputs {
    touchscreen: bool,
    pen: bool,
    touchpad: bool,
    precision_touchpad: bool,
    pointing_stick: bool,
    mouse: bool,
    ps2_mouse: bool,
    lid: bool,
    tablet_switch: bool,
    tablet_mode: bool,
    accelerometer: bool,
}

/// One input device, classified as udev's `input_id` builtin does, from
/// its capability bitmaps.
fn classify_input(event: &Path, inputs: &mut Inputs) {
    let device = event.join("device");
    // Virtual devices (keyd, ydotool, uinput test rigs) are not hardware.
    if fs::canonicalize(&device)
        .map(|path| path.to_string_lossy().contains("/devices/virtual/"))
        .unwrap_or(false)
    {
        return;
    }
    let caps = device.join("capabilities");
    let ev = Bits::parse(&read(&caps.join("ev")));
    let key = Bits::parse(&read(&caps.join("key")));
    let abs = Bits::parse(&read(&caps.join("abs")));
    let rel = Bits::parse(&read(&caps.join("rel")));
    let sw = Bits::parse(&read(&caps.join("sw")));
    let properties = Bits::parse(&read(&device.join("properties")));
    let name = read(&device.join("name")).to_ascii_lowercase();
    let bus = hex(&read(&device.join("id/bustype"))).unwrap_or(0);

    if ev.has(EV_SW) {
        inputs.lid |= sw.has(SW_LID);
        if sw.has(SW_TABLET_MODE) {
            inputs.tablet_switch = true;
            // The switch's current state is not in sysfs; the
            // compositor reads it. Leave tablet_mode for Windows.
        }
    }
    if properties.has(INPUT_PROP_ACCELEROMETER) {
        // HP's lis3lv02d protects the disk; it cannot rotate the screen.
        inputs.accelerometer |= !name.contains("lis3lv02");
        return;
    }
    let absolute = ev.has(EV_ABS) && abs.has(ABS_X) && abs.has(ABS_Y);
    let multitouch = ev.has(EV_ABS) && abs.has(ABS_MT_POSITION_X);
    let pen = key.has(BTN_TOOL_PEN) || key.has(BTN_STYLUS);
    let finger = key.has(BTN_TOOL_FINGER);
    let direct = properties.has(INPUT_PROP_DIRECT);
    if ev.has(EV_KEY) && (absolute || multitouch) {
        if pen && !finger {
            inputs.pen = true;
        } else if finger && !direct {
            inputs.touchpad = true;
            // Two-finger reporting: libinput's gestures and two-finger
            // scrolling, what Windows calls a precision touchpad.
            inputs.precision_touchpad |= multitouch && key.has(BTN_TOOL_DOUBLETAP);
        } else if key.has(BTN_TOUCH) && (direct || multitouch) {
            inputs.touchscreen = true;
            inputs.pen |= pen;
        }
    }
    let relative = ev.has(EV_REL) && rel.has(REL_X) && rel.has(REL_Y) && key.has(BTN_MOUSE);
    if relative {
        if properties.has(INPUT_PROP_POINTING_STICK)
            || name.contains("trackpoint")
            || name.contains("pointing stick")
            || name.contains("pointstick")
        {
            inputs.pointing_stick = true;
        } else if bus == BUS_USB || bus == BUS_BLUETOOTH {
            inputs.mouse = true;
        } else if name == "ps/2 generic mouse" {
            // Some firmware presents the built-in stick as a generic PS/2
            // mouse (EliteBook 840 G2): decided below, beside a touchpad.
            inputs.ps2_mouse = true;
        } else if !name.contains("touchpad") && !name.contains("synaptics") {
            inputs.mouse |= bus != 0x11 && bus != 0x18 && bus != 0x19;
        }
    }
}

fn usb_fingerprint(vendor: u32, product: u32, description: &str) -> bool {
    let description = description.to_ascii_lowercase();
    description.contains("fingerprint")
        || description.contains("finger print")
        || matches!(
            vendor,
            // Validity/Synaptics (USB parts are sensors), Goodix, FPC,
            // UPEK, AuthenTec, EgisTec, Next Biometrics, Chipsailing,
            // FocalTech's sensors.
            0x138a | 0x06cb | 0x27c6 | 0x10a5 | 0x147e | 0x08ff | 0x1c7a | 0x298d | 0x2541 | 0x2808
        )
        // ELAN's USB fingerprint sensors; its other USB ids are screens.
        || (vendor == 0x04f3 && (0x0c00..=0x0cff).contains(&product))
}

/// Refresh rate (mHz) and physical size (mm) from an EDID's first
/// detailed timing descriptor.
pub(crate) fn edid_timing(edid: &[u8]) -> (Option<u32>, Option<(u32, u32)>) {
    if edid.len() < 128 || edid[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return (None, None);
    }
    let d = &edid[54..72];
    let clock_khz = u32::from(u16::from_le_bytes([d[0], d[1]])) * 10;
    if clock_khz == 0 {
        return (None, None);
    }
    let h_active = u32::from(d[2]) | (u32::from(d[4] >> 4) << 8);
    let h_blank = u32::from(d[3]) | (u32::from(d[4] & 0xf) << 8);
    let v_active = u32::from(d[5]) | (u32::from(d[7] >> 4) << 8);
    let v_blank = u32::from(d[6]) | (u32::from(d[7] & 0xf) << 8);
    let total = u64::from(h_active + h_blank) * u64::from(v_active + v_blank);
    let refresh = (total > 0).then(|| (u64::from(clock_khz) * 1_000_000 / total) as u32);
    let width_mm = u32::from(d[12]) | (u32::from(d[14] >> 4) << 8);
    let height_mm = u32::from(d[13]) | (u32::from(d[14] & 0xf) << 8);
    let size = (width_mm > 0 && height_mm > 0).then_some((width_mm, height_mm));
    (refresh, size)
}

fn displays(sys: &Path) -> Vec<Display> {
    let mut found = Vec::new();
    for path in entries(&sys.join("class/drm")) {
        let name = file_name(&path);
        let Some((_, connector)) = name.split_once('-') else {
            continue;
        };
        if read(&path.join("status")) != "connected" {
            continue;
        }
        let internal = ["eDP", "LVDS", "DSI", "DPI"]
            .iter()
            .any(|kind| connector.starts_with(kind));
        let mode = read(&path.join("modes"));
        let (width, height) = mode
            .lines()
            .next()
            .and_then(|line| line.split_once('x'))
            .and_then(|(w, h)| {
                let h: String = h.chars().take_while(char::is_ascii_digit).collect();
                Some((w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or((0, 0));
        let edid = fs::read(path.join("edid")).unwrap_or_default();
        let (refresh, size) = edid_timing(&edid);
        let ppi = size
            .filter(|_| width > 0)
            .map(|(width_mm, _)| (f64::from(width) * 25.4 / f64::from(width_mm)).round() as u32);
        found.push(Display {
            name: connector.to_string(),
            internal,
            width,
            height,
            refresh_millihertz: refresh,
            scale: None,
            ppi,
        });
    }
    found.sort_by_key(|display| (!display.internal, display.name.clone()));
    found
}

fn gpus(sys: &Path) -> Vec<crate::model::Gpu> {
    let mut found = Vec::new();
    for card in entries(&sys.join("class/drm")) {
        let name = file_name(&card);
        if !name
            .strip_prefix("card")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let device = card.join("device");
        let (Some(vendor), Some(id)) = (
            hex(&read(&device.join("vendor"))),
            hex(&read(&device.join("device"))),
        ) else {
            continue;
        };
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .map(|path| file_name(&path))
            .unwrap_or_default();
        let vram = read(&device.join("mem_info_vram_total"))
            .parse::<u64>()
            .ok();
        let label = if driver.is_empty() {
            format!("{vendor:04x}:{id:04x}")
        } else {
            format!("{driver} {vendor:04x}:{id:04x}")
        };
        found.push(gpu::classify(vendor, id, &label, vram, driver == "radeon"));
    }
    found
}

fn cpu(sys: &Path, proc_dir: &Path) -> (u32, u32) {
    let mut threads = 0;
    let mut cores = BTreeSet::new();
    for path in entries(&sys.join("devices/system/cpu")) {
        let name = file_name(&path);
        if !name
            .strip_prefix("cpu")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        if read(&path.join("online")) == "0" {
            continue;
        }
        threads += 1;
        let topology = path.join("topology");
        cores.insert((
            read(&topology.join("physical_package_id")),
            read(&topology.join("core_id")),
            read(&topology.join("die_id")),
        ));
    }
    if threads == 0 {
        // No sysfs topology (some containers): count /proc/cpuinfo.
        threads = read(&proc_dir.join("cpuinfo"))
            .lines()
            .filter(|line| line.starts_with("processor"))
            .count() as u32;
        return (threads, threads);
    }
    (cores.len() as u32, threads)
}

fn memory(proc_dir: &Path) -> u64 {
    read(&proc_dir.join("meminfo"))
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

/// Read everything under `roots`. The result is not yet classified.
pub fn detect(roots: &Roots) -> HardwareProfile {
    let sys = &roots.sys;
    let mut profile = HardwareProfile::default();

    for path in entries(&sys.join("class/power_supply")) {
        if read(&path.join("type")) == "Battery" && read(&path.join("scope")) != "Device" {
            profile.batteries += 1;
        }
    }
    profile.backlight = !entries(&sys.join("class/backlight")).is_empty();
    profile.keyboard_backlight = entries(&sys.join("class/leds"))
        .iter()
        .any(|path| file_name(path).contains("kbd_backlight"));
    profile.bluetooth = entries(&sys.join("class/bluetooth"))
        .iter()
        .any(|path| !file_name(path).contains(':'));
    for path in entries(&sys.join("class/net")) {
        let name = file_name(&path);
        if name == "lo" {
            continue;
        }
        let uevent = read(&path.join("uevent"));
        let devtype = uevent
            .lines()
            .find_map(|line| line.strip_prefix("DEVTYPE="))
            .unwrap_or_default();
        let physical = path.join("device").exists();
        if path.join("wireless").exists() || path.join("phy80211").exists() || devtype == "wlan" {
            profile.wifi = true;
        } else if devtype == "wwan" || name.starts_with("wwan") {
            profile.cellular = true;
        } else if physical && read(&path.join("type")) == "1" && devtype.is_empty() {
            profile.ethernet = true;
        }
    }
    profile.cellular |= !entries(&sys.join("class/wwan")).is_empty();

    let mut inputs = Inputs::default();
    for path in entries(&sys.join("class/input")) {
        let name = file_name(&path);
        if name
            .strip_prefix("event")
            .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_digit()))
        {
            classify_input(&path, &mut inputs);
        }
    }
    if inputs.ps2_mouse {
        if inputs.touchpad {
            inputs.pointing_stick = true;
        } else {
            inputs.mouse = true;
        }
    }

    for path in entries(&sys.join("bus/iio/devices")) {
        let name = read(&path.join("name")).to_ascii_lowercase();
        if path.join("in_illuminance_raw").exists()
            || path.join("in_illuminance_input").exists()
            || name == "als"
        {
            profile.ambient_light_sensor = true;
        }
        if !name.contains("lis3lv02")
            && path.join("in_accel_x_raw").exists()
            && path.join("in_accel_y_raw").exists()
            && path.join("in_accel_z_raw").exists()
        {
            inputs.accelerometer = true;
        }
    }

    for path in entries(&sys.join("bus/usb/devices")) {
        let (Some(vendor), Some(product)) = (
            hex(&read(&path.join("idVendor"))),
            hex(&read(&path.join("idProduct"))),
        ) else {
            continue;
        };
        if usb_fingerprint(vendor, product, &read(&path.join("product"))) {
            profile.fingerprint.present = true;
        }
    }
    profile.fingerprint = Fingerprint {
        present: profile.fingerprint.present,
        service: profile.fingerprint.present
            && (roots.root.join("usr/libexec/fprintd").exists()
                || roots
                    .root
                    .join("usr/share/dbus-1/system-services/net.reactivated.Fprint.service")
                    .exists()),
    };

    let mut cameras = 0;
    for path in entries(&sys.join("class/video4linux")) {
        if !file_name(&path).starts_with("video") || read(&path.join("index")) != "0" {
            continue;
        }
        let name = read(&path.join("name")).to_ascii_lowercase();
        if name.contains(" ir ") || name.contains("ir camera") || name.ends_with(" ir") {
            profile.ir_camera = true;
        } else {
            cameras += 1;
        }
    }
    profile.webcam = cameras > 0;

    for path in entries(&sys.join("class/sound")) {
        let name = file_name(&path);
        if name.starts_with("pcmC") {
            profile.speakers |= name.ends_with('p');
            profile.microphone |= name.ends_with('c');
        }
    }

    let chassis = read(&sys.join("class/dmi/id/chassis_type"))
        .parse::<u32>()
        .unwrap_or(0);
    profile.touchscreen = inputs.touchscreen;
    profile.pen = inputs.pen;
    profile.touchpad = inputs.touchpad;
    profile.precision_touchpad = inputs.precision_touchpad;
    profile.pointing_stick = inputs.pointing_stick;
    profile.mouse = inputs.mouse;
    profile.accelerometer = inputs.accelerometer;
    profile.convertible = inputs.tablet_switch || matches!(chassis, 30..=32);
    profile.tablet_mode = inputs.tablet_mode;
    profile.portable = inputs.lid || profile.batteries > 0 || matches!(chassis, 8..=14 | 30..=32);

    profile.displays = displays(sys);
    profile.gpus = gpus(sys);
    let (cores, threads) = cpu(sys, &roots.proc_dir);
    profile.cpu_cores = cores;
    profile.cpu_threads = threads;
    profile.memory_bytes = memory(&roots.proc_dir);
    profile
}
