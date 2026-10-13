use serde::{Deserialize, Serialize};

/// Everything Lulo adapts to on this PC. Built by the platform detectors
/// (`linux::detect`, `windows::detect`) and pure classifiers over what they
/// read, so each rule is tested with fake device lists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HardwareProfile {
    /// Batteries that power the PC (not a mouse's or a headset's).
    pub batteries: u32,
    /// The internal panel's brightness can be set (a backlight).
    pub backlight: bool,
    pub touchscreen: bool,
    /// Simultaneous touch points the screen reports, where the platform says
    /// (Windows' `SM_MAXIMUMTOUCHES`; Linux needs the device node to know).
    pub max_touch_points: Option<u32>,
    /// A pen or stylus digitiser.
    pub pen: bool,
    pub touchpad: bool,
    /// A Windows Precision Touchpad (or, on Linux, a multi-touch touchpad
    /// libinput drives with gestures).
    pub precision_touchpad: bool,
    pub pointing_stick: bool,
    /// A mouse other than the built-in touchpad or stick (USB or Bluetooth).
    pub mouse: bool,
    pub fingerprint: Fingerprint,
    /// An infrared camera for face sign-in (Windows Hello face).
    pub ir_camera: bool,
    pub webcam: bool,
    /// Something that records sound (an internal or plugged-in microphone).
    pub microphone: bool,
    /// Something that plays sound (speakers, headphones, HDMI audio).
    pub speakers: bool,
    pub bluetooth: bool,
    pub wifi: bool,
    pub ethernet: bool,
    pub cellular: bool,
    pub ambient_light_sensor: bool,
    /// An orientation sensor (an accelerometer that can rotate the screen;
    /// HP's disk-protection sensor does not count).
    pub accelerometer: bool,
    /// The PC folds or detaches into a tablet (a tablet-mode switch, or the
    /// firmware's convertible/detachable chassis).
    pub convertible: bool,
    /// The PC is in tablet mode right now.
    pub tablet_mode: bool,
    pub keyboard_backlight: bool,
    /// A lid switch, a battery or a laptop chassis.
    pub portable: bool,
    pub displays: Vec<Display>,
    pub gpus: Vec<Gpu>,
    /// Installed memory in bytes, as the OS reports it.
    pub memory_bytes: u64,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
    pub spec: SpecClass,
    /// Why `spec` is what it is, in words (empty for a standard PC).
    pub spec_reasons: Vec<String>,
    /// How long detection took, in microseconds.
    pub detection_micros: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fingerprint {
    /// A fingerprint sensor is attached.
    pub present: bool,
    /// The platform's sign-in stack can use it: Windows Hello's biometric
    /// service on Windows, fprintd installed on Linux. Whether a finger is
    /// enrolled is the settings pane's question (it asks the service).
    pub service: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// The connector (`eDP-1`, `HDMI-A-1`) or Windows' device name.
    pub name: String,
    pub internal: bool,
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz (60 000 for 60 Hz), when known.
    pub refresh_millihertz: Option<u32>,
    /// The scale the OS uses now (Windows' DPI ÷ 96), when known.
    pub scale: Option<f32>,
    /// Pixels per inch from the panel's physical size, when known.
    pub ppi: Option<u32>,
}

impl Display {
    /// High density: a scale of 1.5 or more, or 190 ppi or more.
    pub fn hidpi(&self) -> bool {
        self.scale.is_some_and(|scale| scale >= 1.5) || self.ppi.is_some_and(|ppi| ppi >= 190)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuVendor {
    Intel,
    Amd,
    Nvidia,
    Apple,
    Qualcomm,
    /// A virtual machine's or the OS's software adapter.
    Virtual,
    #[default]
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gpu {
    pub vendor: GpuVendor,
    /// PCI vendor and device ids.
    pub vendor_id: u32,
    pub device_id: u32,
    /// The driver's or the adapter's own name.
    pub name: String,
    pub integrated: bool,
    /// An integrated GPU from before 2015 (Intel Haswell and older, AMD's
    /// `radeon`-driver generations): blur and big animations cost frames.
    pub old: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecClass {
    /// 4 GB of memory or less, four CPU threads or fewer, or an old
    /// integrated GPU: Lulo turns down blur and heavy motion by itself.
    Low,
    #[default]
    Standard,
}

const GIB: u64 = 1 << 30;

/// The low-spec rule (ADR 0027). Memory is "4 GB or less" as sold: the OS
/// reports a little less than what is installed, so the cut is 4.5 GiB.
pub fn classify(profile: &HardwareProfile) -> (SpecClass, Vec<String>) {
    let mut reasons = Vec::new();
    if profile.memory_bytes > 0 && profile.memory_bytes <= 4 * GIB + GIB / 2 {
        reasons.push(format!(
            "{:.1} GB of memory",
            profile.memory_bytes as f64 / GIB as f64
        ));
    }
    if profile.cpu_threads > 0 && profile.cpu_threads <= 4 {
        reasons.push(format!("{} CPU threads", profile.cpu_threads));
    }
    let main_gpu = profile
        .gpus
        .iter()
        .find(|gpu| !gpu.integrated)
        .or(profile.gpus.first());
    if let Some(gpu) = main_gpu.filter(|gpu| gpu.old) {
        reasons.push(format!("an old integrated GPU ({})", gpu.name));
    }
    let class = if reasons.is_empty() {
        SpecClass::Standard
    } else {
        SpecClass::Low
    };
    (class, reasons)
}

impl HardwareProfile {
    /// Set `spec` and `spec_reasons` from the rest.
    pub fn classify(mut self) -> Self {
        let (spec, reasons) = classify(&self);
        self.spec = spec;
        self.spec_reasons = reasons;
        self
    }

    pub fn has_battery(&self) -> bool {
        self.batteries > 0
    }

    /// The internal panel, if there is one.
    pub fn internal_display(&self) -> Option<&Display> {
        self.displays.iter().find(|display| display.internal)
    }

    /// What Lulo shows and turns down on this PC.
    pub fn adaptations(&self) -> Adaptations {
        let low = self.spec == SpecClass::Low;
        Adaptations {
            battery: self.has_battery(),
            wifi: self.wifi,
            bluetooth: self.bluetooth,
            display_brightness: self.backlight,
            keyboard_brightness: self.keyboard_backlight,
            // AirDrop-like sharing needs both radios, as on the Mac.
            nearby_sharing: self.wifi && self.bluetooth,
            touch_targets: self.touchscreen,
            fingerprint: self.fingerprint.present && self.fingerprint.service,
            reduce_transparency: low,
            reduce_motion: low,
        }
    }

    /// One line for the log: what was found and how long it took.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!(
            "{} cores/{} threads, {:.1} GB",
            self.cpu_cores,
            self.cpu_threads,
            self.memory_bytes as f64 / GIB as f64
        ));
        for gpu in &self.gpus {
            parts.push(format!(
                "{} GPU {}{}",
                if gpu.vendor == GpuVendor::Virtual {
                    "virtual"
                } else if gpu.integrated {
                    "integrated"
                } else {
                    "discrete"
                },
                gpu.name,
                if gpu.old { " (old)" } else { "" }
            ));
        }
        for display in &self.displays {
            let mut text = format!(
                "{} display {} {}x{}",
                if display.internal {
                    "internal"
                } else {
                    "external"
                },
                display.name,
                display.width,
                display.height
            );
            if let Some(refresh) = display.refresh_millihertz {
                text.push_str(&format!(" @{:.0}Hz", refresh as f64 / 1000.0));
            }
            if let Some(scale) = display.scale {
                text.push_str(&format!(" scale {scale:.2}"));
            }
            if display.hidpi() {
                text.push_str(" HiDPI");
            }
            parts.push(text);
        }
        let flags = [
            (self.batteries > 0, format!("battery x{}", self.batteries)),
            (self.backlight, "backlight".into()),
            (
                self.touchscreen,
                match self.max_touch_points {
                    Some(points) => format!("touchscreen ({points} points)"),
                    None => "touchscreen".into(),
                },
            ),
            (self.pen, "pen".into()),
            (
                self.touchpad,
                if self.precision_touchpad {
                    "precision touchpad".into()
                } else {
                    "touchpad".into()
                },
            ),
            (self.pointing_stick, "pointing stick".into()),
            (self.mouse, "mouse".into()),
            (
                self.fingerprint.present,
                if self.fingerprint.service {
                    "fingerprint".into()
                } else {
                    "fingerprint (no sign-in service)".into()
                },
            ),
            (self.ir_camera, "IR camera".into()),
            (self.webcam, "webcam".into()),
            (self.microphone, "microphone".into()),
            (self.speakers, "speakers".into()),
            (self.bluetooth, "Bluetooth".into()),
            (self.wifi, "Wi-Fi".into()),
            (self.ethernet, "Ethernet".into()),
            (self.cellular, "cellular".into()),
            (self.ambient_light_sensor, "light sensor".into()),
            (self.accelerometer, "accelerometer".into()),
            (self.convertible, "convertible".into()),
            (self.tablet_mode, "tablet mode".into()),
            (self.keyboard_backlight, "keyboard backlight".into()),
        ];
        parts.extend(
            flags
                .into_iter()
                .filter(|(present, _)| *present)
                .map(|(_, name)| name),
        );
        let spec = match self.spec {
            SpecClass::Low => format!("low-spec ({})", self.spec_reasons.join(", ")),
            SpecClass::Standard => "standard spec".into(),
        };
        format!(
            "Lulo hardware: {spec}; {} [{} ms]",
            parts.join(", "),
            self.detection_micros.div_ceil(1000)
        )
    }
}

/// What the shell does with a profile. Each surface asks for its own flag
/// rather than reading devices, so the rules live here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Adaptations {
    /// Menu bar: the battery item.
    pub battery: bool,
    /// Control Centre and the menu bar: Wi-Fi.
    pub wifi: bool,
    /// Control Centre and the menu bar: Bluetooth.
    pub bluetooth: bool,
    /// Control Centre: the Display brightness slider.
    pub display_brightness: bool,
    /// Control Centre: Keyboard Brightness.
    pub keyboard_brightness: bool,
    /// Control Centre: AirDrop-like sharing.
    pub nearby_sharing: bool,
    /// Larger hit targets for fingers.
    pub touch_targets: bool,
    /// Lock screen and Touch ID-style settings: offer the fingerprint.
    pub fingerprint: bool,
    /// Low-spec: solid materials instead of blur.
    pub reduce_transparency: bool,
    /// Low-spec: skip heavy animations.
    pub reduce_motion: bool,
}

impl Default for Adaptations {
    /// Before detection finishes: show everything, turn nothing down, so a
    /// slow first detection never hides a control the PC has.
    fn default() -> Self {
        Self {
            battery: true,
            wifi: true,
            bluetooth: true,
            display_brightness: true,
            keyboard_brightness: true,
            nearby_sharing: true,
            touch_targets: false,
            fingerprint: true,
            reduce_transparency: false,
            reduce_motion: false,
        }
    }
}
