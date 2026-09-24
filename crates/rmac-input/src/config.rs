//! Niri configuration discovery, traversal, and effective-state parsing.

use super::*;

#[derive(Debug)]
pub(super) struct ConfigLocation {
    pub(super) path: PathBuf,
    pub(super) writable_user_config: bool,
}

#[derive(Debug)]
pub(super) struct Authority {
    pub(super) main_path: PathBuf,
    pub(super) main_source: String,
    pub(super) managed_path: PathBuf,
    pub(super) managed_source: Option<String>,
    pub(super) has_managed_include: bool,
    pub(super) safe_to_write: bool,
    pub(super) detail: Option<String>,
    pub(super) effective: EffectiveConfig,
    pub(super) files: Vec<ConfigFile>,
    pub(super) missing_optional_files: Vec<PathBuf>,
}

#[derive(Debug)]
pub(super) struct ConfigFile {
    pub(super) path: PathBuf,
    pub(super) source: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct EffectiveConfig {
    pub(super) settings: InputSettings,
    pub(super) mouse_node: Option<KdlNode>,
    pub(super) touchpad_node: Option<KdlNode>,
    pub(super) xkb_from_include: bool,
}

#[derive(Default)]
pub(super) struct GraphState {
    pub(super) effective: EffectiveConfig,
    pub(super) files: Vec<ConfigFile>,
    pub(super) missing_optional_files: Vec<PathBuf>,
    pub(super) total_bytes: u64,
    pub(super) stack: HashSet<PathBuf>,
}

#[cfg(target_os = "linux")]
pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use notify::Watcher as _;

    let callback = sender.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let update = match event {
            Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => return,
            Ok(_) => WatchEvent::Changed,
            Err(error) => WatchEvent::WatchError(error.to_string()),
        };
        let _ = callback.try_send(update);
    })
    .map_err(|error| Error::new("watch Linux input devices", error.to_string()))?;
    watcher
        .watch(Path::new("/dev/input"), notify::RecursiveMode::NonRecursive)
        .map_err(|error| Error::new("watch Linux input devices", error.to_string()))?;
    sender.closed().await;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) async fn system_watch(_: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    Err(Error::new(
        "watch input devices",
        "live input-device watching is available only on Linux",
    ))
}

pub(super) fn config_path() -> Result<Option<ConfigLocation>, Error> {
    if let Some(path) = std::env::var_os("NIRI_CONFIG")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        if !path.is_absolute() {
            return Err(Error::new(
                "resolve the niri configuration",
                "NIRI_CONFIG must be an absolute path",
            ));
        }
        return Ok(path.is_file().then_some(ConfigLocation {
            path,
            writable_user_config: true,
        }));
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("resolve the niri configuration", "HOME is not set"))?;
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("niri/config.kdl"),
        _ => home.join(".config/niri/config.kdl"),
    };
    if path.is_file() {
        return Ok(Some(ConfigLocation {
            path,
            writable_user_config: true,
        }));
    }
    let system = PathBuf::from("/etc/niri/config.kdl");
    Ok(system.is_file().then_some(ConfigLocation {
        path: system,
        writable_user_config: false,
    }))
}

pub(super) fn load_authority(location: &ConfigLocation) -> Result<Authority, Error> {
    let main_path = normalize_absolute(&location.path)?;
    let mut graph = GraphState::default();
    traverse_config(&main_path, 0, &mut graph)?;
    let main_source = graph
        .files
        .first()
        .map(|file| file.source.clone())
        .ok_or_else(|| Error::new("read the niri configuration", "the graph is empty"))?;
    let main = KdlDocument::parse_v1(&main_source)
        .map_err(|error| Error::new("parse the niri configuration", format!("{error}")))?;
    let managed_path = main_path.with_file_name(MANAGED_CONFIG_NAME);
    let mut managed_include_indexes = Vec::new();
    for (index, node) in main.nodes().iter().enumerate() {
        if node.name().value() == "include" && include_path(node, &main_path)?.path == managed_path
        {
            managed_include_indexes.push((index, node));
        }
    }
    let has_managed_include = managed_include_indexes.len() == 1
        && managed_include_indexes[0].0 + 1 == main.nodes().len()
        && managed_include_indexes[0].1.len() == 1
        && managed_include_indexes[0].1.children().is_none()
        && managed_include_indexes[0]
            .1
            .get(0)
            .and_then(KdlValue::as_string)
            == Some(MANAGED_CONFIG_NAME);
    let managed_graph_references = graph
        .files
        .iter()
        .filter(|file| file.path == managed_path)
        .count()
        + graph
            .missing_optional_files
            .iter()
            .filter(|path| **path == managed_path)
            .count();
    let include_is_ambiguous = managed_graph_references != usize::from(has_managed_include)
        || managed_include_indexes.len() > 1
        || managed_include_indexes
            .first()
            .is_some_and(|(index, node)| {
                *index + 1 != main.nodes().len()
                    || node.len() != 1
                    || node.children().is_some()
                    || node.get(0).and_then(KdlValue::as_string) != Some(MANAGED_CONFIG_NAME)
            });

    let managed_source = match std::fs::symlink_metadata(&managed_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => None,
        Ok(_) => Some(read_bounded_config(&managed_path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Error::new(
                "inspect the managed input configuration",
                error.to_string(),
            ));
        }
    };
    let managed_is_owned = managed_source.as_deref().is_none_or(|source| {
        source
            .strip_prefix(MANAGED_HEADER)
            .is_some_and(|body| body.is_empty() || body.starts_with('\n'))
    });
    if managed_is_owned {
        if let Some(source) = managed_source.as_deref() {
            parse_managed_document(source)?;
        }
    }

    if has_managed_include
        && managed_source.as_deref().is_some_and(|managed| {
            graph
                .files
                .iter()
                .filter(|file| file.path == managed_path)
                .any(|file| file.source != managed)
        })
    {
        return Err(Error::new(
            "read the managed input configuration",
            "the managed file changed while the include graph was being read",
        ));
    }
    let main_is_symlink = std::fs::symlink_metadata(&main_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true);
    let safe_to_write = location.writable_user_config
        && !main_is_symlink
        && !include_is_ambiguous
        && managed_is_owned
        && !std::fs::symlink_metadata(&managed_path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
    let detail = if !location.writable_user_config {
        Some("niri is using the system configuration; create a user configuration before changing input settings.".into())
    } else if main_is_symlink {
        Some(
            "The main niri configuration is a symbolic link and will not be replaced by Lulo OS."
                .into(),
        )
    } else if include_is_ambiguous {
        Some(
            "The Lulo OS input include must appear exactly once as the final top-level niri node."
                .into(),
        )
    } else if !managed_is_owned {
        Some(format!(
            "{} exists but is not owned by Lulo OS.",
            managed_path.display()
        ))
    } else if !safe_to_write {
        Some(
            "The managed input configuration is a symbolic link and will not be replaced by Lulo OS."
                .into(),
        )
    } else if !has_managed_include {
        Some("Input values are effective from the current include graph and have not yet been placed under isolated Lulo OS ownership.".into())
    } else {
        None
    };
    Ok(Authority {
        main_path,
        main_source,
        managed_path,
        managed_source,
        has_managed_include,
        safe_to_write,
        detail,
        effective: graph.effective,
        files: graph.files,
        missing_optional_files: graph.missing_optional_files,
    })
}

pub(super) fn snapshot_from_authority(authority: Authority, available: bool) -> Snapshot {
    let keyboard_layout_authority = if !available {
        KeyboardLayoutAuthority::Unavailable
    } else if authority.effective.settings.keyboard.xkb_override.is_some() {
        if authority.effective.xkb_from_include {
            KeyboardLayoutAuthority::IncludedConfig
        } else {
            KeyboardLayoutAuthority::NiriConfig
        }
    } else {
        KeyboardLayoutAuthority::SystemLocaled
    };
    let (devices, device_detail) = system_devices();
    Snapshot {
        available,
        can_configure: available && authority.safe_to_write,
        config_path: Some(authority.main_path),
        detail: if available {
            authority.detail
        } else {
            Some("Input controls require the niri compositor on Linux.".into())
        },
        keyboard_layout_authority,
        settings: authority.effective.settings,
        included_files: authority.files.len().saturating_sub(1),
        devices,
        device_detail,
        device_overrides: DeviceOverrideCapability::Unsupported,
    }
}

pub(super) fn apply_input(input: &KdlDocument, effective: &mut EffectiveConfig) {
    if let Some(keyboard) = input.get("keyboard").and_then(KdlNode::children) {
        effective.settings.keyboard.repeat_delay_ms = integer(keyboard, "repeat-delay")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(effective.settings.keyboard.repeat_delay_ms);
        effective.settings.keyboard.repeat_rate = integer(keyboard, "repeat-rate")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(effective.settings.keyboard.repeat_rate);
        if let Some(value) = flag(keyboard, "numlock") {
            effective.settings.keyboard.numlock = value;
        }
        if let Some(xkb) = keyboard.get("xkb").and_then(KdlNode::children) {
            let override_ = XkbOverride {
                rules: string(xkb, "rules").unwrap_or_default().to_owned(),
                layout: string(xkb, "layout").unwrap_or_default().to_owned(),
                model: string(xkb, "model").unwrap_or_default().to_owned(),
                variant: string(xkb, "variant").unwrap_or_default().to_owned(),
                options: string(xkb, "options").map(str::to_owned),
                file: string(xkb, "file").map(str::to_owned),
            };
            effective.settings.keyboard.xkb_override =
                (override_ != XkbOverride::default()).then_some(override_);
        }
    }
    if let Some(mouse) = input.get("mouse") {
        effective.settings.mouse = PointerSettings::default();
        if let Some(children) = mouse.children() {
            read_pointer(children, &mut effective.settings.mouse);
        }
        effective.mouse_node = Some(mouse.clone());
    }
    if let Some(touchpad_node) = input.get("touchpad") {
        effective.settings.touchpad = TouchpadSettings::default();
        if let Some(touchpad) = touchpad_node.children() {
            read_pointer(touchpad, &mut effective.settings.touchpad.pointer);
            effective.settings.touchpad.tap_to_click = flag(touchpad, "tap").unwrap_or(false);
            effective.settings.touchpad.disable_while_typing =
                flag(touchpad, "dwt").unwrap_or(false);
            effective.settings.touchpad.drag_lock = flag(touchpad, "drag-lock").unwrap_or(false);
            effective.settings.touchpad.secondary_click = string(touchpad, "click-method")
                .and_then(SecondaryClick::from_id)
                .unwrap_or_default();
        }
        effective.touchpad_node = Some(touchpad_node.clone());
    }
}

pub(super) fn traverse_config(
    path: &Path,
    depth: usize,
    state: &mut GraphState,
) -> Result<(), Error> {
    if depth >= MAX_INCLUDE_DEPTH {
        return Err(Error::new(
            "traverse the niri include graph",
            "the include graph exceeds niri's ten-level recursion limit",
        ));
    }
    let path = normalize_absolute(path)?;
    if !state.stack.insert(path.clone()) {
        return Err(Error::new(
            "traverse the niri include graph",
            format!("recursive include detected at {}", path.display()),
        ));
    }
    if state.files.len() >= MAX_CONFIG_FILES {
        return Err(Error::new(
            "traverse the niri include graph",
            "the include graph exceeds the 64-file safety limit",
        ));
    }
    let source = read_bounded_config(&path)?;
    state.total_bytes = state
        .total_bytes
        .checked_add(source.len() as u64)
        .ok_or_else(|| Error::new("traverse the niri include graph", "size overflow"))?;
    if state.total_bytes > MAX_GRAPH_BYTES {
        return Err(Error::new(
            "traverse the niri include graph",
            "the include graph exceeds the 8 MiB safety limit",
        ));
    }
    state.files.push(ConfigFile {
        path: path.clone(),
        source: source.clone(),
    });
    let document = KdlDocument::parse_v1(&source).map_err(|error| {
        Error::new(
            "parse the niri include graph",
            format!("{}: {error}", path.display()),
        )
    })?;
    let mut saw_input = false;
    for node in document.nodes() {
        match node.name().value() {
            "include" => {
                let include = include_path(node, &path)?;
                match std::fs::metadata(&include.path) {
                    Ok(_) => traverse_config(&include.path, depth + 1, state)?,
                    Err(error)
                        if include.optional && error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        state.missing_optional_files.push(include.path);
                    }
                    Err(error) => {
                        return Err(Error::new(
                            "read the niri include graph",
                            format!("{}: {error}", include.path.display()),
                        ));
                    }
                }
            }
            "input" => {
                if saw_input {
                    return Err(Error::new(
                        "parse the niri include graph",
                        format!(
                            "{} contains more than one top-level input block",
                            path.display()
                        ),
                    ));
                }
                saw_input = true;
                if let Some(input) = node.children() {
                    let sets_xkb = input
                        .get("keyboard")
                        .and_then(KdlNode::children)
                        .is_some_and(|keyboard| keyboard.get("xkb").is_some());
                    apply_input(input, &mut state.effective);
                    if sets_xkb {
                        state.effective.xkb_from_include =
                            depth > 0 && state.effective.settings.keyboard.xkb_override.is_some();
                    }
                }
            }
            _ => {}
        }
    }
    state.stack.remove(&path);
    Ok(())
}

pub(super) struct IncludePath {
    pub(super) path: PathBuf,
    pub(super) optional: bool,
}

pub(super) fn include_path(node: &KdlNode, source_path: &Path) -> Result<IncludePath, Error> {
    let raw = node.get(0).and_then(KdlValue::as_string).ok_or_else(|| {
        Error::new(
            "parse the niri include graph",
            format!(
                "{} has an include without a string path",
                source_path.display()
            ),
        )
    })?;
    if node.children().is_some() || node.len() != usize::from(node.get("optional").is_some()) + 1 {
        return Err(Error::new(
            "parse the niri include graph",
            format!(
                "{} has an include with unsupported arguments",
                source_path.display()
            ),
        ));
    }
    let optional = match node.get("optional") {
        Some(value) => value.as_bool().ok_or_else(|| {
            Error::new(
                "parse the niri include graph",
                format!(
                    "{} has a non-boolean optional include",
                    source_path.display()
                ),
            )
        })?,
        None => false,
    };
    let raw_path = PathBuf::from(raw);
    let expanded = if let Ok(rest) = raw_path.strip_prefix("~") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| Error::new("expand a niri include", "HOME is not set"))?;
        home.join(rest)
    } else if raw_path.is_absolute() {
        raw_path
    } else {
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join(raw_path)
    };
    Ok(IncludePath {
        path: normalize_absolute(&expanded)?,
        optional,
    })
}

pub(super) fn normalize_absolute(path: &Path) -> Result<PathBuf, Error> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(Error::new(
            "normalize the niri configuration path",
            format!("{} is not absolute", path.display()),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(Error::new(
                        "normalize the niri configuration path",
                        "the path escapes the filesystem root",
                    ));
                }
            }
            Component::Normal(component) => normalized.push(component),
        }
    }
    Ok(normalized)
}

pub(super) fn read_bounded_config(path: &Path) -> Result<String, Error> {
    let file = std::fs::File::open(path).map_err(|error| {
        Error::new(
            "read the niri input configuration",
            format!("{}: {error}", path.display()),
        )
    })?;
    if file
        .metadata()
        .map_err(|error| Error::new("inspect the niri input configuration", error.to_string()))?
        .len()
        > MAX_CONFIG_BYTES
    {
        return Err(Error::new(
            "read the niri input configuration",
            format!("{} exceeds the 2 MiB safety limit", path.display()),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new("read the niri input configuration", error.to_string()))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(Error::new(
            "read the niri input configuration",
            format!("{} exceeds the 2 MiB safety limit", path.display()),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        Error::new(
            "read the niri input configuration",
            format!("{} is not valid UTF-8", path.display()),
        )
    })
}

#[cfg(target_os = "linux")]
pub(super) fn system_devices() -> (Vec<InputDevice>, Option<String>) {
    const MAX_DEVICES: usize = 128;
    let directory = Path::new("/sys/class/input");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Kernel input devices could not be read: {error}")),
            );
        }
    };
    let mut devices = Vec::new();
    let mut truncated = false;
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !id.strip_prefix("event").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
        }) {
            continue;
        }
        if devices.len() == MAX_DEVICES {
            truncated = true;
            break;
        }
        let device = entry.path().join("device");
        let name = read_small_text(&device.join("name"), 4096)
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Unnamed input device".into());
        let properties = read_small_text(&entry.path().join("dev"), 64)
            .and_then(|dev| {
                let udev = Path::new("/run/udev/data").join(format!("c{}", dev.trim()));
                read_small_text(&udev, 64 * 1024)
            })
            .unwrap_or_default();
        let kind = classify_device(&name, &properties);
        devices.push(InputDevice { id, name, kind });
    }
    devices.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    let detail = truncated.then(|| "Only the first 128 kernel input devices are shown.".into());
    (devices, detail)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn system_devices() -> (Vec<InputDevice>, Option<String>) {
    (
        Vec::new(),
        Some("Live input-device inventory is available only on Linux.".into()),
    )
}

#[cfg(target_os = "linux")]
pub(super) fn read_small_text(path: &Path, limit: u64) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn classify_device(name: &str, properties: &str) -> DeviceKind {
    let property = |key: &str| {
        let expected = format!("E:{key}=1");
        properties.lines().any(|line| line == expected)
    };
    if property("ID_INPUT_TOUCHPAD") {
        DeviceKind::Touchpad
    } else if property("ID_INPUT_TOUCHSCREEN") {
        DeviceKind::Touchscreen
    } else if property("ID_INPUT_TABLET") {
        DeviceKind::Tablet
    } else if property("ID_INPUT_POINTINGSTICK") {
        DeviceKind::Trackpoint
    } else if property("ID_INPUT_TRACKBALL") {
        DeviceKind::Trackball
    } else if property("ID_INPUT_MOUSE") {
        DeviceKind::Mouse
    } else if property("ID_INPUT_KEYBOARD") {
        DeviceKind::Keyboard
    } else {
        let name = name.to_ascii_lowercase();
        if name.contains("touchpad") {
            DeviceKind::Touchpad
        } else if name.contains("touchscreen") {
            DeviceKind::Touchscreen
        } else if name.contains("trackpoint") || name.contains("pointing stick") {
            DeviceKind::Trackpoint
        } else if name.contains("trackball") {
            DeviceKind::Trackball
        } else if name.contains("mouse") {
            DeviceKind::Mouse
        } else if name.contains("keyboard") || name.contains("kbd") {
            DeviceKind::Keyboard
        } else {
            DeviceKind::Other
        }
    }
}

pub(super) fn read_pointer(document: &KdlDocument, pointer: &mut PointerSettings) {
    pointer.enabled = !flag(document, "off").unwrap_or(false);
    pointer.natural_scroll = flag(document, "natural-scroll").unwrap_or(false);
    pointer.left_handed = flag(document, "left-handed").unwrap_or(false);
    pointer.middle_emulation = flag(document, "middle-emulation").unwrap_or(false);
    pointer.accel_speed = number(document, "accel-speed").unwrap_or(pointer.accel_speed);
    pointer.accel_profile = match string(document, "accel-profile") {
        Some("flat") => AccelProfile::Flat,
        _ => AccelProfile::Adaptive,
    };
}

pub(super) fn flag(document: &KdlDocument, name: &str) -> Option<bool> {
    document
        .nodes()
        .iter()
        .rev()
        .find(|node| node.name().value() == name)
        .map(|node| node.get(0).and_then(KdlValue::as_bool).unwrap_or(true))
}

pub(super) fn integer(document: &KdlDocument, name: &str) -> Option<i128> {
    document.get_arg(name).and_then(KdlValue::as_integer)
}

pub(super) fn number(document: &KdlDocument, name: &str) -> Option<f64> {
    document.get_arg(name).and_then(|value| {
        value
            .as_float()
            .or_else(|| value.as_integer().map(|n| n as f64))
    })
}

pub(super) fn string<'a>(document: &'a KdlDocument, name: &str) -> Option<&'a str> {
    document.get_arg(name).and_then(KdlValue::as_string)
}
