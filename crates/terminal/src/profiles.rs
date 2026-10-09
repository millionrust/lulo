use std::path::{Path, PathBuf};

use crate::storage;

/// A macOS Terminal-style color profile: chrome colors plus the ANSI palette.
#[derive(Clone, Copy)]
pub(crate) struct Profile {
    pub(crate) name: &'static str,
    pub(crate) bg: u32,
    pub(crate) fg: u32,
    pub(crate) cursor: u32,
    pub(crate) selection: u32,
    /// ANSI colors: indices 0–7 normal and 8–15 bright.
    pub(crate) ansi: [u32; 16],
}

const MAC_ANSI: [u32; 16] = [
    0x000000, 0x990000, 0x00a600, 0x999900, 0x0000b2, 0xb200b2, 0x00a6b2, 0xbfbfbf, 0x666666,
    0xe50000, 0x00d900, 0xe5e500, 0x0000ff, 0xe500e5, 0x00e5e5, 0xe5e5e5,
];

const ONE_DARK: [u32; 16] = [
    0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf, 0x5c6370,
    0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
];

/// Stable built-in profiles. Index zero is the rmac default.
pub(crate) static PROFILES: &[Profile] = &[
    Profile {
        name: "Lulo OS Dark",
        bg: 0x1e1e1e,
        fg: 0xd4d4d4,
        cursor: 0xd4d4d4,
        selection: 0x2f5d8c,
        ansi: ONE_DARK,
    },
    Profile {
        name: "Basic",
        bg: 0xffffff,
        fg: 0x000000,
        cursor: 0x000000,
        selection: 0xb4d5fe,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Pro",
        bg: 0x000000,
        fg: 0xf2f2f2,
        cursor: 0x4d4d4d,
        selection: 0x414141,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Homebrew",
        bg: 0x000000,
        fg: 0x00ff00,
        cursor: 0x23ff18,
        selection: 0x083905,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Grass",
        bg: 0x13773d,
        fg: 0xfff0a5,
        cursor: 0x8c1543,
        selection: 0x004d00,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Man Page",
        bg: 0xfef49c,
        fg: 0x000000,
        cursor: 0x7f7f7f,
        selection: 0xa3d7ff,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Novel",
        bg: 0xdfdbc3,
        fg: 0x3b2322,
        cursor: 0x73635a,
        selection: 0xa4a390,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Ocean",
        bg: 0x224fbc,
        fg: 0xffffff,
        cursor: 0x7f7f7f,
        selection: 0x216dff,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Red Sands",
        bg: 0x7a251e,
        fg: 0xd7c9a7,
        cursor: 0xffffff,
        selection: 0xa4a390,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Clear Dark",
        bg: 0x1e1e1e,
        fg: 0xffffff,
        cursor: 0x9c9d9d,
        selection: 0x464646,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Clear Light",
        bg: 0xffffff,
        fg: 0x000000,
        cursor: 0x000000,
        selection: 0xb4d5fe,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Silver Aerogel",
        bg: 0xe8e8e8,
        fg: 0x222222,
        cursor: 0x555555,
        selection: 0xa6c8eb,
        ansi: MAC_ANSI,
    },
    Profile {
        name: "Solid Colors",
        bg: 0x2b3344,
        fg: 0xffffff,
        cursor: 0xffffff,
        selection: 0x56657e,
        ansi: MAC_ANSI,
    },
];

/// Index of "Basic", Terminal's default profile.
pub(crate) const DEFAULT_PROFILE: usize = 1;

/// Basic follows the system appearance on macOS. In dark mode it measures
/// #1E1E1E behind white text with a #9C9D9D block cursor (macOS 26.2); the
/// selection colour is not measured (S) and uses the system's dark
/// unemphasised selection grey.
static BASIC_DARK: Profile = Profile {
    name: "Basic",
    bg: 0x1e1e1e,
    fg: 0xffffff,
    cursor: 0x9c9d9d,
    selection: 0x464646,
    ansi: MAC_ANSI,
};

thread_local! {
    static ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(DEFAULT_PROFILE) };
    /// Shell ▸ Edit Background Colour (⌥⌘I): this window's live override,
    /// published right before `render` the same way `set_active` already
    /// publishes its profile index — never polled, just read back by
    /// `active()` during that one render.
    static ACTIVE_BACKGROUND_OVERRIDE: std::cell::Cell<Option<u32>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn set_active(index: usize) {
    ACTIVE.with(|active| active.set(index));
}

pub(crate) fn set_active_background_override(colour: Option<u32>) {
    ACTIVE_BACKGROUND_OVERRIDE.with(|cell| cell.set(colour));
}

/// The active profile's colours as this window should actually draw them
/// right now: `resolved(index)`, with Shell ▸ Edit Background Colour's live
/// override (if any) replacing `bg`. Unlike `resolved`, which is pure
/// profile data for pickers/previews, this reflects one window's own
/// override — so a window that hasn't set one still draws the profile's
/// own colour exactly as before.
pub(crate) fn active() -> Profile {
    let index = ACTIVE.with(|active| active.get());
    let mut profile = *resolved(index);
    if let Some(bg) = ACTIVE_BACKGROUND_OVERRIDE.with(std::cell::Cell::get) {
        profile.bg = bg;
    }
    profile
}

/// The profile at `index` as drawn in the current appearance: Basic swaps to
/// its dark colours while rmac is dark, like Terminal's appearance-aware
/// Basic profile.
pub(crate) fn resolved(index: usize) -> &'static Profile {
    let profile = PROFILES.get(index).unwrap_or(&PROFILES[DEFAULT_PROFILE]);
    if index == DEFAULT_PROFILE && rmac_ui::mac::window().l < 0.5 {
        &BASIC_DARK
    } else {
        profile
    }
}

/// Edit ▸ Copy Special ▸ Style for "Copy" Command (TRM-MENU-001..015): which
/// colours a plain Edit ▸ Copy renders its styled (HTML/RTF) clipboard
/// content with, independent of what the window is actually displaying.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyStyle {
    /// "Terminal's Settings (Default)": the window's own live profile —
    /// what you see is what a styled paste carries.
    Default,
    /// "Plain Text": Copy carries no styling at all, like Copy Plain Text.
    PlainText,
    /// One specific named built-in profile, by its index into `PROFILES`,
    /// regardless of which profile the window is actually showing.
    Profile(usize),
}

thread_local! {
    static COPY_STYLE: std::cell::Cell<CopyStyle> = const { std::cell::Cell::new(CopyStyle::Default) };
}

pub(crate) fn set_copy_style(style: CopyStyle) {
    COPY_STYLE.with(|cell| cell.set(style));
}

pub(crate) fn copy_style() -> CopyStyle {
    COPY_STYLE.with(|cell| cell.get())
}

/// Terminal ▸ Settings… ▸ Text ▸ "Use bright colours for bold text", applied
/// to plain (non-ANSI-coloured) bold text: lightens `rgb` toward white by
/// the same fraction regardless of how dark or light it starts, so it stays
/// a visibly bolder shade of the same colour rather than clipping to white.
pub(crate) fn brighten(rgb: u32) -> u32 {
    const MIX: f32 = 0.35;
    let channel = |shift: u32| {
        let value = ((rgb >> shift) & 0xff) as f32;
        (value + (255.0 - value) * MIX).round().clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn config_path() -> Result<PathBuf, storage::Failure> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            Path::new("profile.txt"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let directory = home.join("Library/Application Support/rmac-terminal");
    #[cfg(not(target_os = "macos"))]
    let directory = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path.join("rmac-terminal"),
        _ => home.join(".config/rmac-terminal"),
    };
    Ok(directory.join("profile.txt"))
}

fn parse(content: &str) -> Result<(usize, bool), String> {
    let value = content.trim();
    if value.is_empty() {
        return Err("profile preference is empty".into());
    }
    if let Some(index) = PROFILES.iter().position(|profile| profile.name == value) {
        return Ok((index, false));
    }
    if let Ok(index) = value.parse::<usize>() {
        return (index < PROFILES.len())
            .then_some((index, true))
            .ok_or_else(|| format!("legacy profile index {index} is out of range"));
    }
    Err(format!("unknown terminal profile '{value}'"))
}

pub(crate) fn load() -> Result<(usize, bool), storage::Failure> {
    let path = config_path()?;
    match storage::load_optional(
        &storage::RealStorage,
        &path,
        storage::Operation::LoadProfile,
    )? {
        Some(content) => parse(&content).map_err(|detail| {
            storage::Failure::message(storage::Operation::LoadProfile, &path, detail)
        }),
        None => Ok((DEFAULT_PROFILE, false)),
    }
}

pub(crate) fn save(index: usize) -> Result<(), storage::Failure> {
    let path = config_path()?;
    let profile = PROFILES.get(index).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::SaveProfile,
            &path,
            format!("profile index {index} is out of range"),
        )
    })?;
    storage::save(
        &storage::RealStorage,
        &path,
        profile.name,
        storage::Operation::SaveProfile,
    )
}

fn setting_path(name: &str) -> Result<PathBuf, storage::Failure> {
    Ok(config_path()?.with_file_name(name))
}

/// Shell ▸ Use Option as Meta Key: off by default, like the Mac.
pub(crate) fn load_option_as_meta() -> bool {
    (|| -> Result<bool, storage::Failure> {
        let path = setting_path("option-as-meta.txt")?;
        let stored = storage::load_optional(
            &storage::RealStorage,
            &path,
            storage::Operation::LoadSetting,
        )?;
        Ok(stored.is_some_and(|value| value.trim() == "1"))
    })()
    .unwrap_or(false)
}

pub(crate) fn save_option_as_meta(enabled: bool) -> Result<(), storage::Failure> {
    let path = setting_path("option-as-meta.txt")?;
    storage::save(
        &storage::RealStorage,
        &path,
        if enabled { "1" } else { "0" },
        storage::Operation::SaveSetting,
    )
}

/// The font size new Terminal windows open with, from Settings.
pub(crate) fn load_font_size() -> Option<f32> {
    let path = setting_path("font-size.txt").ok()?;
    let stored = storage::load_optional(
        &storage::RealStorage,
        &path,
        storage::Operation::LoadSetting,
    )
    .ok()
    .flatten()?;
    stored
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|size| (8.0..=32.0).contains(size))
}

pub(crate) fn save_font_size(size: f32) -> Result<(), storage::Failure> {
    let path = setting_path("font-size.txt")?;
    storage::save(
        &storage::RealStorage,
        &path,
        format!("{size}"),
        storage::Operation::SaveSetting,
    )
}

/// Shell ▸ Edit Background Colour (⌥⌘I): overrides the active profile's `bg`
/// for new windows, like `load_font_size`/`save_font_size` override the
/// profile's font size. `None` means "use the profile's own colour".
pub(crate) fn load_background_override() -> Option<u32> {
    let path = setting_path("background-colour.txt").ok()?;
    let stored = storage::load_optional(
        &storage::RealStorage,
        &path,
        storage::Operation::LoadSetting,
    )
    .ok()
    .flatten()?;
    u32::from_str_radix(stored.trim().trim_start_matches('#'), 16).ok()
}

pub(crate) fn save_background_override(colour: u32) -> Result<(), storage::Failure> {
    let path = setting_path("background-colour.txt")?;
    storage::save(
        &storage::RealStorage,
        &path,
        format!("{:06x}", colour & 0x00ff_ffff),
        storage::Operation::SaveSetting,
    )
}

/// Edit ▸ Marks ▸ Automatically Mark Prompt Lines: on by default, like the
/// Mac. Turning it off stops `shell_integration`'s OSC 133 handler from
/// recording a prompt mark for Edit ▸ Navigate's Jump/Select to
/// Previous/Next Mark — manual marks (⌘U) and bookmarks are unaffected.
pub(crate) fn load_automatically_mark_prompt_lines() -> bool {
    (|| -> Result<bool, storage::Failure> {
        let path = setting_path("auto-mark-prompts.txt")?;
        let stored = storage::load_optional(
            &storage::RealStorage,
            &path,
            storage::Operation::LoadSetting,
        )?;
        Ok(stored.is_none_or(|value| value.trim() != "0"))
    })()
    .unwrap_or(true)
}

pub(crate) fn save_automatically_mark_prompt_lines(enabled: bool) -> Result<(), storage::Failure> {
    let path = setting_path("auto-mark-prompts.txt")?;
    storage::save(
        &storage::RealStorage,
        &path,
        if enabled { "1" } else { "0" },
        storage::Operation::SaveSetting,
    )
}

#[cfg(test)]
mod tests {
    use super::{brighten, parse, PROFILES};

    #[test]
    fn brighten_lightens_every_channel_toward_white() {
        assert_eq!(brighten(0x000000), 0x595959);
        assert_eq!(brighten(0xffffff), 0xffffff);
        assert_eq!(brighten(0xd4d4d4), 0xe3e3e3);
        // Mid-tone colours brighten without any channel overshooting 0xff
        // or a lighter channel ending up darker than a darker one started.
        let dark = brighten(0x101010);
        let light = brighten(0xe0e0e0);
        assert!(dark < light);
        assert!(dark <= 0xffffff);
    }

    #[test]
    fn all_mac_profile_names_are_selectable() {
        for name in [
            "Basic",
            "Clear Dark",
            "Clear Light",
            "Grass",
            "Homebrew",
            "Man Page",
            "Novel",
            "Ocean",
            "Pro",
            "Red Sands",
            "Silver Aerogel",
            "Solid Colors",
        ] {
            assert!(
                PROFILES.iter().any(|profile| profile.name == name),
                "{name}"
            );
        }
    }

    #[test]
    fn stable_names_and_legacy_indices_are_supported() {
        for (index, profile) in PROFILES.iter().enumerate() {
            assert_eq!(parse(profile.name), Ok((index, false)));
            assert_eq!(parse(&index.to_string()), Ok((index, true)));
        }
    }

    #[test]
    fn malformed_or_unknown_profiles_are_reported() {
        assert!(parse("").is_err());
        assert!(parse("Not a Profile").is_err());
        assert!(parse(&PROFILES.len().to_string()).is_err());
    }
}
