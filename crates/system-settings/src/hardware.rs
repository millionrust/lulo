//! Small, bounded Linux hardware inventory. All calls run on a background executor.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DisplayDevice {
    pub name: String,
    pub internal: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub has_battery: bool,
    pub has_touchpad: bool,
    pub has_pointing_stick: bool,
    pub has_external_mouse: bool,
    pub has_touchscreen: bool,
    pub has_pen: bool,
    pub has_backlight: bool,
    pub has_keyboard_backlight: bool,
    pub has_fingerprint: bool,
    pub fprintd_supported: bool,
    pub has_bluetooth: bool,
    pub has_wifi: bool,
    pub displays: Vec<DisplayDevice>,
    pub audio_outputs: Vec<String>,
    pub has_lid: bool,
    pub has_accelerometer: bool,
    pub rotation_capable: bool,
}

impl Capabilities {
    pub fn allows_pane(&self, name: &str) -> bool {
        match name {
            "Battery" => self.has_battery,
            "Trackpad" => self.has_touchpad,
            "Mouse" => self.has_external_mouse || self.has_pointing_stick,
            "Bluetooth" => self.has_bluetooth,
            "Wi-Fi" => self.has_wifi,
            "Touchscreen" => self.has_touchscreen,
            _ => true,
        }
    }
}

fn entries(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .map(|items| items.flatten().take(256).map(|item| item.path()).collect())
        .unwrap_or_default()
}

fn read(path: &Path) -> String {
    fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() <= 64 * 1024)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn any_entry(path: &Path) -> bool {
    !entries(path).is_empty()
}

pub(crate) fn scan(sys: &Path, udev: &Path, fprintd_supported: bool) -> Capabilities {
    let mut result = Capabilities::default();
    let mut integrated_ps2_mouse = false;
    for path in entries(&sys.join("class/power_supply")) {
        result.has_battery |= read(&path.join("type")) == "Battery";
    }
    result.has_backlight = any_entry(&sys.join("class/backlight"));
    result.has_bluetooth = any_entry(&sys.join("class/bluetooth"));
    for path in entries(&sys.join("class/leds")) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        result.has_keyboard_backlight |= name.contains("kbd_backlight");
        result.has_accelerometer |= name.contains("hddprotect");
    }
    for path in entries(&sys.join("class/net")) {
        result.has_wifi |= path.join("wireless").exists() || path.join("phy80211").exists();
    }
    for path in entries(&sys.join("class/drm")) {
        if read(&path.join("status")) != "connected" {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let internal = name.contains("-eDP-") || name.contains("-LVDS-") || name.contains("-DSI-");
        result.displays.push(DisplayDevice { name, internal });
    }
    result.displays.sort_by(|a, b| a.name.cmp(&b.name));
    for path in entries(&sys.join("class/sound")) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with("pcm") && name.ends_with('p') {
            result.audio_outputs.push(name);
        }
    }
    result.audio_outputs.sort();
    for path in entries(&sys.join("class/input")) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !name.starts_with("event") || !name[5..].chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let device_name = read(&path.join("device/name")).to_ascii_lowercase();
        let dev = read(&path.join("dev"));
        let properties = read(&udev.join(format!("c{dev}")));
        let has = |key: &str| properties.lines().any(|line| line == format!("E:{key}=1"));
        result.has_touchpad |= has("ID_INPUT_TOUCHPAD") || device_name.contains("touchpad");
        result.has_pointing_stick |= has("ID_INPUT_POINTINGSTICK")
            || device_name.contains("trackpoint")
            || device_name.contains("pointing stick");
        result.has_touchscreen |=
            has("ID_INPUT_TOUCHSCREEN") || device_name.contains("touchscreen");
        result.has_pen |= has("ID_INPUT_TABLET")
            || has("ID_INPUT_TABLET_PAD")
            || device_name.contains("stylus")
            || device_name.contains("pen digitizer");
        result.has_lid |= has("ID_INPUT_SWITCH") && device_name.contains("lid")
            || device_name.contains("lid switch");
        let mouse = has("ID_INPUT_MOUSE") || device_name.contains("mouse");
        let ps2_candidate = mouse
            && properties.lines().any(|line| line == "E:ID_BUS=i8042")
            && device_name == "ps/2 generic mouse";
        integrated_ps2_mouse |= ps2_candidate;
        result.has_external_mouse |= mouse
            && !ps2_candidate
            && !has("ID_INPUT_POINTINGSTICK")
            && !device_name.contains("trackpoint")
            && !device_name.contains("pointing stick")
            && !device_name.contains("touchpad");
    }
    // Some firmware presents the built-in stick as a generic PS/2 mouse,
    // with no ID_INPUT_POINTINGSTICK udev property (EliteBook 840 G2).
    if integrated_ps2_mouse && result.has_touchpad {
        result.has_pointing_stick = true;
    } else if integrated_ps2_mouse {
        result.has_external_mouse = true;
    }
    for path in entries(&sys.join("bus/usb/devices")) {
        let vendor = read(&path.join("idVendor"));
        let product = read(&path.join("idProduct"));
        let description = read(&path.join("product")).to_ascii_lowercase();
        result.has_fingerprint |= (vendor == "138a" && product == "003f")
            || description.contains("fingerprint")
            || description.contains("finger print");
    }
    result.fprintd_supported = result.has_fingerprint && fprintd_supported;
    for path in entries(&sys.join("bus/iio/devices")) {
        let name = read(&path.join("name")).to_ascii_lowercase();
        let acceleration = name.contains("accel") || path.join("in_accel_x_raw").exists();
        result.has_accelerometer |= acceleration;
        // HP's lis3lv02d sensor protects the hard disk; it does not expose
        // display orientation. Require three axes and exclude that device.
        result.rotation_capable |= acceleration
            && !name.contains("lis3lv02d")
            && path.join("in_accel_x_raw").exists()
            && path.join("in_accel_y_raw").exists()
            && path.join("in_accel_z_raw").exists();
    }
    result
}

#[cfg(target_os = "linux")]
pub(crate) fn current() -> Capabilities {
    let mut result = scan(Path::new("/sys"), Path::new("/run/udev/data"), false);
    if !result.has_fingerprint {
        return result;
    }
    let fprintd_supported = std::process::Command::new("busctl")
        .args([
            "--system",
            "--timeout=2s",
            "call",
            "net.reactivated.Fprint",
            "/net/reactivated/Fprint/Manager",
            "net.reactivated.Fprint.Manager",
            "GetDevices",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|output| {
            output
                .split_whitespace()
                .nth(1)
                .and_then(|count| count.parse::<usize>().ok())
                .is_some_and(|count| count > 0)
        });
    result.fprintd_supported = fprintd_supported;
    result
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn current() -> Capabilities {
    Capabilities::default()
}

/// Wait for kernel uevents. The child is killed when Settings drops the
/// receiver, and no timer or periodic filesystem scan runs while idle.
#[cfg(target_os = "linux")]
pub(crate) async fn watch(sender: async_channel::Sender<()>) -> std::io::Result<()> {
    use std::io::{BufRead as _, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

    let mut child = Command::new("udevadm")
        .args([
            "monitor",
            "--udev",
            "--subsystem-match=input",
            "--subsystem-match=drm",
            "--subsystem-match=power_supply",
            "--subsystem-match=backlight",
            "--subsystem-match=leds",
            "--subsystem-match=bluetooth",
            "--subsystem-match=net",
            "--subsystem-match=sound",
            "--subsystem-match=usb",
            "--subsystem-match=iio",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped monitor stdout");
    let child = Arc::new(Mutex::new(child));
    let events = sender.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) if line.starts_with("UDEV  [") => {
                    if events.is_closed() {
                        break;
                    }
                    let _ = events.try_send(());
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        events.close();
    });
    sender.closed().await;
    let mut child = child.lock().expect("monitor child lock");
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, path: &str, value: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn elitebook_fixture_distinguishes_devices_and_disk_protection() {
        let root = std::env::temp_dir().join(format!(
            "lulo-hardware-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let sys = root.join("sys");
        let udev = root.join("udev");
        put(&sys, "class/power_supply/BAT0/type", "Battery\n");
        put(&sys, "class/backlight/intel_backlight/brightness", "120\n");
        put(&sys, "class/bluetooth/hci0/address", "00:00:00:00:00:00\n");
        put(&sys, "class/net/wlp2s0/wireless/empty", "");
        put(&sys, "class/drm/card0-eDP-1/status", "connected\n");
        put(&sys, "class/drm/card0-HDMI-A-1/status", "disconnected\n");
        put(
            &sys,
            "class/input/event1/device/name",
            "SynPS/2 Synaptics TouchPad\n",
        );
        put(&sys, "class/input/event1/dev", "13:65\n");
        put(&udev, "c13:65", "E:ID_INPUT_TOUCHPAD=1\n");
        put(
            &sys,
            "class/input/event2/device/name",
            "PS/2 Generic Mouse\n",
        );
        put(&sys, "class/input/event2/dev", "13:66\n");
        put(&udev, "c13:66", "E:ID_INPUT_MOUSE=1\nE:ID_BUS=i8042\n");
        put(
            &sys,
            "class/input/event3/device/name",
            "Atmel maXTouch Touchscreen\n",
        );
        put(&sys, "class/input/event3/dev", "13:67\n");
        put(&udev, "c13:67", "E:ID_INPUT_TOUCHSCREEN=1\n");
        put(&sys, "bus/usb/devices/1-2/idVendor", "138a\n");
        put(&sys, "bus/usb/devices/1-2/idProduct", "003f\n");
        put(&sys, "bus/iio/devices/iio:device0/name", "lis3lv02d\n");
        put(&sys, "bus/iio/devices/iio:device0/in_accel_x_raw", "1\n");
        put(&sys, "bus/iio/devices/iio:device0/in_accel_y_raw", "1\n");
        put(&sys, "bus/iio/devices/iio:device0/in_accel_z_raw", "1\n");
        let found = scan(&sys, &udev, false);
        assert!(found.has_battery && found.has_touchpad && found.has_pointing_stick);
        assert!(
            found.has_touchscreen && found.has_backlight && found.has_bluetooth && found.has_wifi
        );
        assert!(found.has_fingerprint && !found.fprintd_supported);
        assert!(scan(&sys, &udev, true).fprintd_supported);
        assert!(!found.has_external_mouse && !found.rotation_capable);
        assert_eq!(found.displays.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_fixture_only_exposes_present_device_panes() {
        let root =
            std::env::temp_dir().join(format!("lulo-hardware-desktop-{}", std::process::id()));
        let sys = root.join("sys");
        let udev = root.join("udev");
        put(
            &sys,
            "class/input/event9/device/name",
            "USB Optical Mouse\n",
        );
        put(&sys, "class/input/event9/dev", "13:9\n");
        put(&udev, "c13:9", "E:ID_INPUT_MOUSE=1\nE:ID_BUS=usb\n");
        put(&sys, "class/drm/card0-HDMI-A-1/status", "connected\n");
        let found = scan(&sys, &udev, false);
        assert!(found.has_external_mouse && found.allows_pane("Mouse"));
        for name in ["Battery", "Trackpad", "Bluetooth", "Wi-Fi", "Touchscreen"] {
            assert!(!found.allows_pane(name), "{name}");
        }
        assert_eq!(found.displays.len(), 1);
        assert!(!found.displays[0].internal);
        fs::remove_dir_all(root).unwrap();
    }
}
