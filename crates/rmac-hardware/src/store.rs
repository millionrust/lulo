//! The profile in Lulo's settings, beside the shell's own settings:
//! `$XDG_CONFIG_HOME/rmac/hardware.json` on Lulo OS (next to `shell.json`),
//! `%APPDATA%\Lulo\hardware.json` on Windows. Written on the first run
//! after install and again when the hardware changes, so System Settings
//! and the session can read it without detecting again.

use std::path::{Path, PathBuf};

use crate::HardwareProfile;

const FILE: &str = "hardware.json";
const MAX_BYTES: usize = 256 * 1024;

/// Where the profile lives for this user; `None` without a home.
pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("LULO_HARDWARE_PROFILE").map(PathBuf::from) {
        return path.is_absolute().then_some(path);
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("Lulo").join(FILE))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        #[cfg(target_os = "macos")]
        let directory = home.join("Library/Application Support/rmac");
        #[cfg(not(target_os = "macos"))]
        let directory = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            Some(xdg) if xdg.is_absolute() => xdg.join("rmac"),
            _ => home.join(".config/rmac"),
        };
        Some(directory.join(FILE))
    }
}

pub fn load_from(path: &Path) -> Option<HardwareProfile> {
    let bytes = rmac_storage::read_bounded_no_follow(path, MAX_BYTES).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_to(path: &Path, profile: &HardwareProfile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let mut json = serde_json::to_vec_pretty(profile).map_err(std::io::Error::other)?;
    json.push(b'\n');
    rmac_storage::atomic_write_private(path, &json)
}

/// The stored profile, if any.
pub fn load() -> Option<HardwareProfile> {
    load_from(&path()?)
}

/// What [`record_at`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// No profile was stored: the first run after install.
    FirstRun,
    /// The hardware differs from the stored profile.
    Changed,
    Unchanged,
    /// The settings could not be written (the reason was logged).
    Failed,
}

/// Store `profile` at `path` unless the same hardware is stored already,
/// logging the one-line summary when it is new or changed.
pub fn record_at(path: &Path, profile: &HardwareProfile) -> Recorded {
    let outcome = match load_from(path) {
        None => Recorded::FirstRun,
        Some(stored) if crate::same_hardware(&stored, profile) => return Recorded::Unchanged,
        Some(_) => Recorded::Changed,
    };
    if let Err(error) = save_to(path, profile) {
        eprintln!(
            "lulo-hardware: could not save the hardware profile to {}: {error}",
            path.display()
        );
        return Recorded::Failed;
    }
    if let Some(niri) = path.parent().map(|parent| parent.join("niri")) {
        write_compositor_include(&niri, profile);
    }
    let line = format!(
        "{}{}",
        profile.summary(),
        if outcome == Recorded::Changed {
            " (hardware changed)"
        } else {
            ""
        }
    );
    eprintln!("{line}");
    // Windows' shell has no console: the line is kept beside the profile
    // too (`hardware.log`), one line per first run or change.
    if let Some(log) = path.parent().map(|parent| parent.join("hardware.log")) {
        append_line(&log, &line);
    }
    outcome
}

fn append_line(path: &Path, line: &str) {
    use std::io::Write as _;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    if let Ok(mut file) = file {
        let _ = writeln!(file, "{line}");
    }
}

/// The lighter blur a low-spec PC gets on Lulo OS.
pub const LOW_SPEC_BLUR: &str =
    "// Written by Lulo (rmac-hardware, ADR 0027): this PC is low-spec, so the
// compositor blurs with one pass instead of three. It is written again only
// when the hardware changes.
blur {
    passes 1
    offset 2
    noise 0.02
    saturation 1.5
}
";

/// The compositor include for this hardware: [`LOW_SPEC_BLUR`] on a
/// low-spec PC, nothing otherwise.
pub fn compositor_include(profile: &HardwareProfile) -> &'static str {
    if profile.adaptations().reduce_transparency {
        LOW_SPEC_BLUR
    } else {
        ""
    }
}

/// `niri/hardware-generated.kdl` beside the settings, which the Lulo OS
/// session includes after the shell's own configuration. Only where that
/// directory exists: a Lulo OS login, not a nested test session.
fn write_compositor_include(niri: &Path, profile: &HardwareProfile) {
    if !niri.is_dir() {
        return;
    }
    let path = niri.join("hardware-generated.kdl");
    if let Err(error) =
        rmac_storage::atomic_write_private(&path, compositor_include(profile).as_bytes())
    {
        eprintln!("lulo-hardware: could not write {}: {error}", path.display());
    }
}

/// [`record_at`] Lulo's own settings path.
pub fn record(profile: &HardwareProfile) -> Recorded {
    match path() {
        Some(path) => record_at(&path, profile),
        None => Recorded::Failed,
    }
}
