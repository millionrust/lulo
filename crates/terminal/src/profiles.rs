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
        name: "rmac Dark",
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
}

pub(crate) fn set_active(index: usize) {
    ACTIVE.with(|active| active.set(index));
}

pub(crate) fn active() -> &'static Profile {
    let index = ACTIVE.with(|active| active.get());
    resolved(index)
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
    match storage::load_optional(&storage::RealStorage, &path)? {
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
    storage::save(&storage::RealStorage, &path, profile.name)
}

#[cfg(test)]
mod tests {
    use super::{parse, PROFILES};

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
