//! Persistent keyboard, mouse, and touchpad settings for a niri session.
//!
//! The service edits only the known nodes inside the existing `input` block.
//! A same-directory candidate is validated by niri before the live config is
//! replaced atomically; niri then applies the change through config hot reload.

use kdl::{KdlDocument, KdlNode, KdlValue};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static CANDIDATE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccelProfile {
    #[default]
    Adaptive,
    Flat,
}

impl AccelProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Adaptive => "Adaptive",
            Self::Flat => "Flat",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Flat => "flat",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardSettings {
    pub repeat_delay_ms: u32,
    pub repeat_rate: u32,
    pub numlock: bool,
    pub xkb_override: Option<XkbOverride>,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            repeat_delay_ms: 600,
            repeat_rate: 25,
            numlock: false,
            xkb_override: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XkbOverride {
    pub layout: String,
    pub model: String,
    pub variant: String,
    pub options: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardLayoutAuthority {
    SystemLocaled,
    NiriConfig,
    IncludedConfig,
    #[default]
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerSettings {
    pub natural_scroll: bool,
    pub accel_speed: f64,
    pub accel_profile: AccelProfile,
    pub left_handed: bool,
}

impl Default for PointerSettings {
    fn default() -> Self {
        Self {
            natural_scroll: false,
            accel_speed: 0.0,
            accel_profile: AccelProfile::Adaptive,
            left_handed: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TouchpadSettings {
    pub pointer: PointerSettings,
    pub tap_to_click: bool,
    pub disable_while_typing: bool,
    pub drag_lock: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputSettings {
    pub keyboard: KeyboardSettings,
    pub mouse: PointerSettings,
    pub touchpad: TouchpadSettings,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub can_configure: bool,
    pub config_path: Option<PathBuf>,
    pub detail: Option<String>,
    pub keyboard_layout_authority: KeyboardLayoutAuthority,
    pub settings: InputSettings,
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Read the effective values represented by the main niri config.
pub fn snapshot() -> Result<Snapshot, Error> {
    let Some(path) = config_path()? else {
        return Ok(Snapshot {
            detail: Some("No existing niri configuration was found.".into()),
            ..Snapshot::default()
        });
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|error| Error::new("read the niri input configuration", error.to_string()))?;
    snapshot_from_source(path, &source)
}

/// Validate and persist all supported input settings as one transaction.
pub fn save(settings: &InputSettings) -> Result<Snapshot, Error> {
    validate_settings(settings)?;
    let current = snapshot()?;
    if !current.available {
        return Err(Error::new(
            "save input settings",
            current
                .detail
                .unwrap_or_else(|| "niri is unavailable".into()),
        ));
    }
    if !current.can_configure {
        return Err(Error::new(
            "save input settings",
            current.detail.unwrap_or_else(|| {
                "the configuration cannot be edited without changing its meaning".into()
            }),
        ));
    }

    let path = current
        .config_path
        .ok_or_else(|| Error::new("resolve the niri configuration", "path is unavailable"))?;
    let source = std::fs::read_to_string(&path)
        .map_err(|error| Error::new("read the niri input configuration", error.to_string()))?;
    let candidate = update_source(&source, settings)?;
    validate_candidate(&path, candidate.as_bytes())?;
    rmac_storage::atomic_write(&path, candidate.as_bytes())
        .map_err(|error| Error::new("replace the niri configuration", error.to_string()))?;
    snapshot_from_source(path, &candidate)
}

fn config_path() -> Result<Option<PathBuf>, Error> {
    if let Some(path) = std::env::var_os("NIRI_CONFIG").map(PathBuf::from) {
        if !path.is_absolute() {
            return Err(Error::new(
                "resolve the niri configuration",
                "NIRI_CONFIG must be an absolute path",
            ));
        }
        return Ok(path.is_file().then_some(path));
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("resolve the niri configuration", "HOME is not set"))?;
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("niri/config.kdl"),
        _ => home.join(".config/niri/config.kdl"),
    };
    Ok(path.is_file().then_some(path))
}

fn snapshot_from_source(path: PathBuf, source: &str) -> Result<Snapshot, Error> {
    let document: KdlDocument = source
        .parse()
        .map_err(|error| Error::new("parse the niri configuration", format!("{error}")))?;
    let has_include = document
        .nodes()
        .iter()
        .any(|node| node.name().value() == "include");
    let settings = read_settings(&document);
    let available =
        cfg!(target_os = "linux") && Command::new("niri").arg("--version").output().is_ok();
    let detail = if has_include {
        Some(
            "This niri config uses includes. Input controls are read-only until included input blocks can be updated safely."
                .into(),
        )
    } else if !available {
        Some("Input controls require the niri compositor on Linux.".into())
    } else {
        None
    };
    let keyboard_layout_authority = if has_include {
        KeyboardLayoutAuthority::IncludedConfig
    } else if settings.keyboard.xkb_override.is_some() {
        KeyboardLayoutAuthority::NiriConfig
    } else if available {
        KeyboardLayoutAuthority::SystemLocaled
    } else {
        KeyboardLayoutAuthority::Unavailable
    };
    Ok(Snapshot {
        available,
        can_configure: available && !has_include,
        config_path: Some(path),
        detail,
        keyboard_layout_authority,
        settings,
    })
}

fn read_settings(document: &KdlDocument) -> InputSettings {
    let mut settings = InputSettings::default();
    let Some(input) = document.get("input").and_then(KdlNode::children) else {
        return settings;
    };
    if let Some(keyboard) = input.get("keyboard").and_then(KdlNode::children) {
        settings.keyboard.repeat_delay_ms = integer(keyboard, "repeat-delay")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(settings.keyboard.repeat_delay_ms);
        settings.keyboard.repeat_rate = integer(keyboard, "repeat-rate")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(settings.keyboard.repeat_rate);
        settings.keyboard.numlock = keyboard.get("numlock").is_some();
        settings.keyboard.xkb_override =
            keyboard
                .get("xkb")
                .and_then(KdlNode::children)
                .map(|xkb| XkbOverride {
                    layout: string(xkb, "layout").unwrap_or_default().to_owned(),
                    model: string(xkb, "model").unwrap_or_default().to_owned(),
                    variant: string(xkb, "variant").unwrap_or_default().to_owned(),
                    options: string(xkb, "options").unwrap_or_default().to_owned(),
                });
    }
    if let Some(mouse) = input.get("mouse").and_then(KdlNode::children) {
        read_pointer(mouse, &mut settings.mouse);
    }
    if let Some(touchpad) = input.get("touchpad").and_then(KdlNode::children) {
        read_pointer(touchpad, &mut settings.touchpad.pointer);
        settings.touchpad.tap_to_click = touchpad.get("tap").is_some();
        settings.touchpad.disable_while_typing = touchpad.get("dwt").is_some();
        settings.touchpad.drag_lock = touchpad.get("drag-lock").is_some();
    }
    settings
}

fn read_pointer(document: &KdlDocument, pointer: &mut PointerSettings) {
    pointer.natural_scroll = document.get("natural-scroll").is_some();
    pointer.left_handed = document.get("left-handed").is_some();
    pointer.accel_speed = number(document, "accel-speed").unwrap_or(pointer.accel_speed);
    pointer.accel_profile = match string(document, "accel-profile") {
        Some("flat") => AccelProfile::Flat,
        _ => AccelProfile::Adaptive,
    };
}

fn integer(document: &KdlDocument, name: &str) -> Option<i128> {
    document.get_arg(name).and_then(KdlValue::as_integer)
}

fn number(document: &KdlDocument, name: &str) -> Option<f64> {
    document.get_arg(name).and_then(|value| {
        value
            .as_float()
            .or_else(|| value.as_integer().map(|n| n as f64))
    })
}

fn string<'a>(document: &'a KdlDocument, name: &str) -> Option<&'a str> {
    document.get_arg(name).and_then(KdlValue::as_string)
}

fn validate_settings(settings: &InputSettings) -> Result<(), Error> {
    if !(100..=2_000).contains(&settings.keyboard.repeat_delay_ms) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat delay must be between 100 and 2000 ms",
        ));
    }
    if !(1..=100).contains(&settings.keyboard.repeat_rate) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat rate must be between 1 and 100 characters per second",
        ));
    }
    for (device, pointer) in [
        ("mouse", &settings.mouse),
        ("touchpad", &settings.touchpad.pointer),
    ] {
        if !pointer.accel_speed.is_finite() || !(-1.0..=1.0).contains(&pointer.accel_speed) {
            return Err(Error::new(
                "validate pointer settings",
                format!("{device} tracking speed must be between -1 and 1"),
            ));
        }
    }
    Ok(())
}

fn update_source(source: &str, settings: &InputSettings) -> Result<String, Error> {
    let mut document: KdlDocument = source
        .parse()
        .map_err(|error| Error::new("parse the niri configuration", format!("{error}")))?;
    let input = ensure_children(&mut document, "input");
    let keyboard = ensure_children(input, "keyboard");
    replace_value(
        keyboard,
        "repeat-delay",
        i128::from(settings.keyboard.repeat_delay_ms),
    );
    replace_value(
        keyboard,
        "repeat-rate",
        i128::from(settings.keyboard.repeat_rate),
    );
    replace_flag(keyboard, "numlock", settings.keyboard.numlock);

    let mouse = ensure_children(input, "mouse");
    write_pointer(mouse, &settings.mouse);

    let touchpad = ensure_children(input, "touchpad");
    write_pointer(touchpad, &settings.touchpad.pointer);
    replace_flag(touchpad, "tap", settings.touchpad.tap_to_click);
    replace_flag(touchpad, "dwt", settings.touchpad.disable_while_typing);
    replace_flag(touchpad, "drag-lock", settings.touchpad.drag_lock);
    Ok(document.to_string())
}

fn ensure_children<'a>(document: &'a mut KdlDocument, name: &str) -> &'a mut KdlDocument {
    if document.get(name).is_none() {
        let mut node = KdlNode::new(name);
        node.set_children(KdlDocument::new());
        document.nodes_mut().push(node);
    }
    let node = document.get_mut(name).expect("node was just created");
    node.ensure_children()
}

fn write_pointer(document: &mut KdlDocument, pointer: &PointerSettings) {
    replace_flag(document, "natural-scroll", pointer.natural_scroll);
    replace_value(document, "accel-speed", pointer.accel_speed);
    replace_value(document, "accel-profile", pointer.accel_profile.id());
    replace_flag(document, "left-handed", pointer.left_handed);
}

fn remove_named(document: &mut KdlDocument, name: &str) {
    document
        .nodes_mut()
        .retain(|node| node.name().value() != name);
}

fn replace_flag(document: &mut KdlDocument, name: &str, enabled: bool) {
    remove_named(document, name);
    if enabled {
        document.nodes_mut().push(KdlNode::new(name));
    }
}

fn replace_value(document: &mut KdlDocument, name: &str, value: impl Into<kdl::KdlEntry>) {
    remove_named(document, name);
    let mut node = KdlNode::new(name);
    node.push(value);
    document.nodes_mut().push(node);
}

fn validate_candidate(live_path: &Path, contents: &[u8]) -> Result<(), Error> {
    let sequence = CANDIDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = live_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.kdl");
    let candidate = live_path.with_file_name(format!(
        ".{file_name}.rmac-{}-{sequence}.candidate",
        std::process::id()
    ));
    rmac_storage::atomic_write(&candidate, contents)
        .map_err(|error| Error::new("write the validation candidate", error.to_string()))?;
    let result = Command::new("niri")
        .arg("--config")
        .arg(&candidate)
        .arg("validate")
        .output();
    let cleanup = std::fs::remove_file(&candidate);
    let output = result.map_err(|error| Error::new("run niri validation", error.to_string()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::new(
            "validate the niri configuration",
            if detail.is_empty() {
                format!("niri exited with {}", output.status)
            } else {
                detail
            },
        ));
    }
    cleanup.map_err(|error| Error::new("remove the validation candidate", error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"// keep this comment
input {
    keyboard {
        repeat-delay 450
        repeat-rate 32
        numlock
        track-layout "global"
    }
    mouse {
        natural-scroll
        accel-speed -0.25
        accel-profile "flat"
    }
    touchpad {
        tap
        dwt
        accel-speed 0.2
    }
    warp-mouse-to-focus
}
binds { Mod+T { spawn "alacritty"; } }
"#;

    #[test]
    fn reads_known_values_and_defaults() {
        let document: KdlDocument = CONFIG.parse().unwrap();
        let settings = read_settings(&document);
        assert_eq!(settings.keyboard.repeat_delay_ms, 450);
        assert_eq!(settings.keyboard.repeat_rate, 32);
        assert!(settings.keyboard.numlock);
        assert!(settings.mouse.natural_scroll);
        assert_eq!(settings.mouse.accel_profile, AccelProfile::Flat);
        assert_eq!(settings.mouse.accel_speed, -0.25);
        assert!(settings.touchpad.tap_to_click);
        assert!(settings.touchpad.disable_while_typing);
        assert_eq!(settings.touchpad.pointer.accel_speed, 0.2);
    }

    #[test]
    fn update_preserves_unknown_nodes_and_turns_flags_off() {
        let mut settings = read_settings(&CONFIG.parse().unwrap());
        settings.keyboard.repeat_rate = 40;
        settings.keyboard.numlock = false;
        settings.mouse.natural_scroll = false;
        settings.touchpad.drag_lock = true;
        let updated = update_source(CONFIG, &settings).unwrap();
        assert!(updated.contains("// keep this comment"));
        assert!(updated.contains("track-layout \"global\""));
        assert!(updated.contains("warp-mouse-to-focus"));
        assert!(updated.contains("Mod+T"));
        let document: KdlDocument = updated.parse().unwrap();
        let reread = read_settings(&document);
        assert_eq!(reread, settings);
        let keyboard = document
            .get("input")
            .unwrap()
            .children()
            .unwrap()
            .get("keyboard")
            .unwrap()
            .children()
            .unwrap();
        assert!(keyboard.get("numlock").is_none());
    }

    #[test]
    fn includes_make_the_snapshot_read_only() {
        let snapshot = snapshot_from_source(
            PathBuf::from("/tmp/config.kdl"),
            "include \"input.kdl\"\ninput {}\n",
        )
        .unwrap();
        assert!(!snapshot.can_configure);
        assert_eq!(
            snapshot.keyboard_layout_authority,
            KeyboardLayoutAuthority::IncludedConfig
        );
        assert!(snapshot.detail.unwrap().contains("includes"));
    }

    #[test]
    fn explicit_xkb_block_owns_keyboard_layout() {
        let source = r#"
input {
    keyboard {
        xkb {
            layout "us,de"
            variant ",nodeadkeys"
            options "grp:alt_shift_toggle"
        }
    }
}
"#;
        let snapshot = snapshot_from_source(PathBuf::from("/tmp/config.kdl"), source).unwrap();
        assert_eq!(
            snapshot.keyboard_layout_authority,
            KeyboardLayoutAuthority::NiriConfig
        );
        let xkb = snapshot.settings.keyboard.xkb_override.unwrap();
        assert_eq!(xkb.layout, "us,de");
        assert_eq!(xkb.variant, ",nodeadkeys");
        assert_eq!(xkb.options, "grp:alt_shift_toggle");
    }

    #[test]
    fn rejects_out_of_range_values() {
        let mut settings = InputSettings::default();
        settings.mouse.accel_speed = 1.1;
        assert!(validate_settings(&settings).is_err());
        settings.mouse.accel_speed = 0.0;
        settings.keyboard.repeat_rate = 0;
        assert!(validate_settings(&settings).is_err());
    }
}
