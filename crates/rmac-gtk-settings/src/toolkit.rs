//! Keep third-party GTK, libadwaita, Qt and browser windows in step with
//! rmac's appearance (FEEL_SPEC.md §D.7 item 3).
//!
//! The session-wide defaults (the `rmac` GTK theme, Inter, the rmac pointer
//! and left-hand window buttons) live in a desktop-specific GSettings override
//! so they apply only inside rmac. This module owns what follows the user's
//! rmac Appearance choice:
//!
//! - `color-scheme` and `accent-color` in `org.gnome.desktop.interface`, which
//!   the Settings portal hands to libadwaita, GTK 4, Firefox, Chromium and Qt;
//! - `~/.config/gtk-{3,4}.0/settings.ini`, because GTK 3 cannot follow the
//!   portal colour scheme and reads `gtk-application-prefer-dark-theme` only
//!   from there, and as the fallback for sandboxed apps without the portal;
//! - managed stubs in `~/.config/gtk-{3,4}.0/gtk.css` that carry the accent and
//!   load the libadwaita stylesheet (libadwaita ignores `gtk-theme`).
//!
//! Files the user replaced, or linked from elsewhere, are left alone.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rmac_theme::{AccentPreference, Preferences, SchemePreference};

use crate::api::{command_error, Error, RealRunner, Runner, SCHEMA};

pub const GTK_THEME: &str = "rmac";
/// Inter at the Mac's 13 px body size, in GTK's 96 dpi points.
pub const FONT_NAME: &str = "Inter 9.75";
pub const CURSOR_THEME: &str = "rmac";
pub const CURSOR_SIZE: u32 = 24;
/// Close, minimise and zoom on the left, in the Mac's order.
pub const DECORATION_LAYOUT: &str = "close,minimize,maximize:";
/// rmac's measured default accent (rmac-design `system_blue`).
pub const DEFAULT_ACCENT: [u8; 3] = [0x13, 0x72, 0xf9];

const COLOR_SCHEME_KEY: &str = "color-scheme";
const ACCENT_COLOR_KEY: &str = "accent-color";
const LIBADWAITA_STYLESHEET: &str = "/usr/share/themes/rmac/libadwaita.css";
const MANAGED_MARKER: &str = "/* rmac: managed file.";

/// GNOME's named accents, anchored on rmac's palette so every Appearance
/// swatch maps to its own name (Graphite is GNOME's slate).
const GNOME_ACCENTS: [(&str, [u8; 3]); 9] = [
    ("blue", [0x13, 0x72, 0xf9]),
    ("teal", [0x30, 0xb0, 0xc7]),
    ("green", [0x34, 0xc7, 0x59]),
    ("yellow", [0xff, 0xcc, 0x00]),
    ("orange", [0xff, 0x95, 0x00]),
    ("red", [0xff, 0x3b, 0x30]),
    ("pink", [0xff, 0x2d, 0x55]),
    ("purple", [0xaf, 0x52, 0xde]),
    ("slate", [0x8e, 0x8e, 0x93]),
];

/// What third-party toolkits were told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolkitAppearance {
    pub dark: bool,
    pub accent: [u8; 3],
}

/// Apply rmac's appearance preferences to third-party toolkits once.
pub fn sync_toolkit_appearance(preferences: &Preferences) -> Result<ToolkitAppearance, Error> {
    let config_home = config_home().ok_or_else(|| {
        Error::new(
            "sync toolkit appearance",
            "neither XDG_CONFIG_HOME nor HOME is an absolute path",
        )
    })?;
    sync_with(
        &RealRunner,
        preferences,
        &config_home,
        Path::new(LIBADWAITA_STYLESHEET).is_file(),
    )
}

/// Follows the rmac theme store and re-applies it whenever the preferences,
/// or the host scheme an Automatic preference follows, change.
#[derive(Default)]
pub struct ToolkitFollower {
    applied: Option<(Preferences, bool)>,
}

impl ToolkitFollower {
    /// Returns `Ok(true)` when it applied a change.
    pub fn poll(&mut self) -> Result<bool, Error> {
        let store = rmac_theme::ThemeStore::from_environment()
            .map_err(|error| Error::new("read Lulo OS appearance", error.operation.to_string()))?;
        let host = rmac_appearance::Snapshot::unavailable(
            "third-party toolkits follow Lulo OS's own preference",
        );
        let preferences = store
            .load(&host)
            .map_err(|error| Error::new("read Lulo OS appearance", error.operation.to_string()))?
            .preferences;
        let host_dark = preferences.color_scheme == SchemePreference::Automatic
            && host_prefers_dark(&RealRunner);
        let key = (preferences, host_dark);
        if self.applied.as_ref() == Some(&key) {
            return Ok(false);
        }
        sync_toolkit_appearance(&key.0)?;
        self.applied = Some(key);
        Ok(true)
    }
}

pub(crate) fn sync_with(
    runner: &impl Runner,
    preferences: &Preferences,
    config_home: &Path,
    stylesheet_installed: bool,
) -> Result<ToolkitAppearance, Error> {
    let accent = accent_rgb(preferences.accent_color);
    let (dark, scheme) = match preferences.color_scheme {
        SchemePreference::Dark => (true, Some("'prefer-dark'")),
        SchemePreference::Light => (false, Some("'prefer-light'")),
        // Automatic follows the host, so rmac must not write it back.
        SchemePreference::Automatic => (host_prefers_dark(runner), None),
    };

    // Files first: they need no GNOME stack, and GTK 3's dark variant
    // depends on them even when gsettings is unavailable.
    let mut first_error = None;
    let mut keep = |result: Result<(), Error>| {
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    };
    keep(write_settings_ini(
        &config_home.join("gtk-3.0/settings.ini"),
        &gtk3_settings(dark),
    ));
    keep(write_settings_ini(
        &config_home.join("gtk-4.0/settings.ini"),
        &gtk4_settings(),
    ));
    if stylesheet_installed {
        keep(write_managed(
            &config_home.join("gtk-3.0/gtk.css"),
            &gtk3_stylesheet(accent),
        ));
        keep(write_managed(
            &config_home.join("gtk-4.0/gtk.css"),
            &gtk4_stylesheet(accent),
        ));
    }
    if let Some(scheme) = scheme {
        keep(set_if_changed(runner, COLOR_SCHEME_KEY, scheme));
    }
    keep(set_if_changed(
        runner,
        ACCENT_COLOR_KEY,
        &format!("'{}'", gnome_accent(accent)),
    ));

    match first_error {
        Some(error) => Err(error),
        None => Ok(ToolkitAppearance { dark, accent }),
    }
}

pub(crate) fn accent_rgb(preference: AccentPreference) -> [u8; 3] {
    match preference {
        AccentPreference::Automatic => DEFAULT_ACCENT,
        AccentPreference::Custom(channels) => {
            channels.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
        }
    }
}

/// The GNOME accent name closest to `rgb`.
pub(crate) fn gnome_accent(rgb: [u8; 3]) -> &'static str {
    let distance = |anchor: [u8; 3]| -> i32 {
        rgb.iter()
            .zip(anchor)
            .map(|(value, anchor)| {
                let delta = i32::from(*value) - i32::from(anchor);
                delta * delta
            })
            .sum()
    };
    GNOME_ACCENTS
        .iter()
        .min_by_key(|(_, anchor)| distance(*anchor))
        .map(|(name, _)| *name)
        .unwrap_or("blue")
}

pub(crate) fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn host_prefers_dark(runner: &impl Runner) -> bool {
    runner
        .run(&["get", SCHEMA, COLOR_SCHEME_KEY])
        .map(|output| output.success && output.stdout.trim() == "'prefer-dark'")
        .unwrap_or(false)
}

/// Write `value` only when the key exists and differs. A missing gsettings or
/// an older schema without the key is not an error: there is nothing to tell.
fn set_if_changed(runner: &impl Runner, key: &str, value: &str) -> Result<(), Error> {
    let current = match runner.run(&["get", SCHEMA, key]) {
        Ok(output) if output.success => output.stdout,
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(command_error("read the desktop appearance", error)),
    };
    if current.trim() == value {
        return Ok(());
    }
    let output = runner
        .run(&["set", SCHEMA, key, value])
        .map_err(|error| command_error("set the desktop appearance", error))?;
    if !output.success {
        return Err(Error::new(
            "set the desktop appearance",
            format!("gsettings rejected {key}"),
        ));
    }
    Ok(())
}

pub(crate) fn gtk3_settings(dark: bool) -> Vec<(&'static str, String)> {
    let mut values = common_settings();
    values.insert(
        1,
        (
            "gtk-application-prefer-dark-theme",
            if dark { "true" } else { "false" }.to_string(),
        ),
    );
    values
}

pub(crate) fn gtk4_settings() -> Vec<(&'static str, String)> {
    // No prefer-dark here: GTK 4 follows the portal colour scheme, and
    // libadwaita warns when the legacy key is set.
    let mut values = common_settings();
    values.push(("gtk-overlay-scrolling", "true".to_string()));
    values
}

fn common_settings() -> Vec<(&'static str, String)> {
    vec![
        ("gtk-theme-name", GTK_THEME.to_string()),
        ("gtk-font-name", FONT_NAME.to_string()),
        ("gtk-cursor-theme-name", CURSOR_THEME.to_string()),
        ("gtk-cursor-theme-size", CURSOR_SIZE.to_string()),
        ("gtk-decoration-layout", DECORATION_LAYOUT.to_string()),
        // A click in a scroll track pages, as on the Mac.
        ("gtk-primary-button-warps-slider", "false".to_string()),
    ]
}

pub(crate) fn gtk3_stylesheet(accent: [u8; 3]) -> String {
    format!(
        "{MANAGED_MARKER} rmac rewrites it when the accent changes; delete this line to keep your own edits. */\n\
         @define-color rmac_accent {};\n",
        hex(accent)
    )
}

pub(crate) fn gtk4_stylesheet(accent: [u8; 3]) -> String {
    let accent = hex(accent);
    format!(
        "{MANAGED_MARKER} rmac rewrites it when the accent changes; delete this line to keep your own edits. */\n\
         @import url(\"file://{LIBADWAITA_STYLESHEET}\");\n\
         \n\
         @define-color rmac_accent {accent};\n\
         :root {{\n  --accent-bg-color: {accent};\n}}\n"
    )
}

/// Replace our keys in the `[Settings]` group and keep everything else.
pub(crate) fn merge_settings_ini(existing: &str, values: &[(&str, String)]) -> String {
    fn flush(out: &mut Vec<String>, values: &[(&str, String)], written: &mut [bool]) {
        for (index, (key, value)) in values.iter().enumerate() {
            if !written[index] {
                out.push(format!("{key}={value}"));
                written[index] = true;
            }
        }
    }

    let mut out = Vec::new();
    let mut written = vec![false; values.len()];
    let mut in_settings = false;
    let mut saw_settings = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_settings {
                flush(&mut out, values, &mut written);
            }
            in_settings = &trimmed[1..trimmed.len() - 1] == "Settings";
            saw_settings |= in_settings;
            out.push(line.to_string());
            continue;
        }
        if in_settings && !trimmed.starts_with('#') && !trimmed.starts_with(';') {
            if let Some((key, _)) = trimmed.split_once('=') {
                if let Some(index) = values.iter().position(|(ours, _)| *ours == key.trim()) {
                    // Rewrite the first occurrence and drop duplicates.
                    if !written[index] {
                        out.push(format!("{}={}", values[index].0, values[index].1));
                        written[index] = true;
                    }
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if in_settings {
        flush(&mut out, values, &mut written);
    } else if !saw_settings {
        if out.last().is_some_and(|line| !line.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[Settings]".to_string());
        flush(&mut out, values, &mut written);
    }
    let mut merged = out.join("\n");
    merged.push('\n');
    merged
}

fn write_settings_ini(path: &Path, values: &[(&str, String)]) -> Result<(), Error> {
    let existing = match read_own_file(path)? {
        Some(existing) => existing,
        None if path.symlink_metadata().is_ok() => return Ok(()),
        None => String::new(),
    };
    let merged = merge_settings_ini(&existing, values);
    if merged == existing {
        return Ok(());
    }
    write_atomic(path, &merged)
}

/// Write a stub we own: absent, or still carrying the managed marker.
fn write_managed(path: &Path, contents: &str) -> Result<(), Error> {
    match read_own_file(path)? {
        Some(existing) if existing == contents => Ok(()),
        Some(existing) if existing.starts_with(MANAGED_MARKER) => write_atomic(path, contents),
        Some(_) => Ok(()),
        None if path.symlink_metadata().is_ok() => Ok(()),
        None => write_atomic(path, contents),
    }
}

/// `Some(contents)` for a regular file, `None` when it is absent or is not a
/// regular file (a symlink into a dotfiles repository, say).
fn read_own_file(path: &Path) -> Result<Option<String>, Error> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_file() => fs::read_to_string(path)
            .map(Some)
            .map_err(|_| Error::new("read GTK settings", "the file is not readable UTF-8")),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::new(
            "read GTK settings",
            "the file cannot be inspected",
        )),
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("write GTK settings", "the path has no parent directory"))?;
    fs::create_dir_all(parent)
        .map_err(|_| Error::new("write GTK settings", "the directory cannot be created"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::new("write GTK settings", "the file name is not UTF-8"))?;
    let temporary = parent.join(format!(".{name}.rmac-{}", std::process::id()));
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|_| {
            let _ = fs::remove_file(&temporary);
            Error::new("write GTK settings", "the file cannot be replaced")
        })
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
}
