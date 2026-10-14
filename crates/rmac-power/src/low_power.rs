//! The Mac's Low Power Mode setting: Never, Always, Only on Battery or Only
//! on Power Adapter, kept as a small preference file and carried out through
//! the system power profile (power-profiles-daemon's `power-saver` on Lulo
//! OS, the Windows power mode on Windows).
//!
//! Settings writes the choice and applies it at once. The shell's status
//! coordinator applies it again whenever the power source changes, so "Only
//! on Battery" follows the adapter while Settings is closed.

use std::path::PathBuf;

use crate::{PowerProfile, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LowPowerMode {
    Never,
    Always,
    OnBattery,
    OnAdapter,
}

impl LowPowerMode {
    pub const ALL: [Self; 4] = [Self::Never, Self::Always, Self::OnBattery, Self::OnAdapter];

    /// The pop-up's wording on macOS 26.
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::Always => "Always",
            Self::OnBattery => "Only on Battery",
            Self::OnAdapter => "Only on Power Adapter",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Always => "always",
            Self::OnBattery => "battery",
            Self::OnAdapter => "adapter",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id.trim())
    }

    /// Whether the choice depends on the power source, which needs the
    /// shell to follow the adapter (Lulo OS only).
    pub fn follows_power_source(self) -> bool {
        matches!(self, Self::OnBattery | Self::OnAdapter)
    }

    /// Whether Low Power should be on now.
    pub fn wants_low_power(self, on_battery: bool) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::OnBattery => on_battery,
            Self::OnAdapter => !on_battery,
        }
    }
}

/// The profile to switch to, if any: Low Power when the mode wants it,
/// otherwise leave any other profile alone and only leave Low Power.
pub fn desired_profile(
    mode: LowPowerMode,
    on_battery: bool,
    active: Option<PowerProfile>,
) -> Option<PowerProfile> {
    let low = active == Some(PowerProfile::PowerSaver);
    match (mode.wants_low_power(on_battery), low) {
        (true, false) => Some(PowerProfile::PowerSaver),
        (false, true) => Some(PowerProfile::Balanced),
        _ => None,
    }
}

fn preference_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        })?;
    Some(base.join("rmac").join("low-power-mode"))
}

/// The saved choice, if the person has made one.
pub fn saved_mode() -> Option<LowPowerMode> {
    let path = preference_path()?;
    let bytes = rmac_storage::read_bounded_no_follow(&path, 64).ok()?;
    LowPowerMode::from_id(std::str::from_utf8(&bytes).ok()?)
}

/// The choice to show: the saved one, or what the current profile says
/// (Low Power on means "Always").
pub fn current_mode(snapshot: &Snapshot) -> LowPowerMode {
    saved_mode().unwrap_or(
        if snapshot.profiles.active == Some(PowerProfile::PowerSaver) {
            LowPowerMode::Always
        } else {
            LowPowerMode::Never
        },
    )
}

pub fn save_mode(mode: LowPowerMode) -> std::io::Result<()> {
    let path = preference_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no configuration directory")
    })?;
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    rmac_storage::atomic_write_private(&path, format!("{}\n", mode.id()).as_bytes())
}

/// Apply `mode` to the system profile for the current power source.
/// Returns whether the profile changed.
pub fn apply(mode: LowPowerMode, snapshot: &Snapshot) -> Result<bool, crate::Error> {
    if !snapshot.profiles.available
        || !snapshot
            .profiles
            .supported
            .contains(&PowerProfile::PowerSaver)
    {
        return Ok(false);
    }
    let on_battery = snapshot
        .battery
        .as_ref()
        .is_some_and(|battery| battery.on_battery);
    match desired_profile(mode, on_battery, snapshot.profiles.active) {
        Some(profile) if snapshot.profiles.supported.contains(&profile) => {
            crate::set_profile(profile)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Carry out the saved choice after the power source changed. A choice
/// that does not depend on the power source is left alone, so a profile
/// picked in Control Centre is not overridden.
pub fn follow_power_source(snapshot: &Snapshot) -> Result<bool, crate::Error> {
    match saved_mode() {
        Some(mode) if mode.follows_power_source() => apply(mode, snapshot),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_on_battery_follows_the_adapter() {
        use PowerProfile::*;
        let mode = LowPowerMode::OnBattery;
        assert_eq!(
            desired_profile(mode, true, Some(Balanced)),
            Some(PowerSaver)
        );
        assert_eq!(desired_profile(mode, true, Some(PowerSaver)), None);
        assert_eq!(
            desired_profile(mode, false, Some(PowerSaver)),
            Some(Balanced)
        );
        // High Power chosen elsewhere stays while Low Power is not wanted.
        assert_eq!(desired_profile(mode, false, Some(Performance)), None);
        assert_eq!(
            desired_profile(LowPowerMode::OnAdapter, false, Some(Balanced)),
            Some(PowerSaver)
        );
        assert_eq!(
            desired_profile(LowPowerMode::Never, true, Some(PowerSaver)),
            Some(Balanced)
        );
        assert_eq!(
            desired_profile(LowPowerMode::Always, false, None),
            Some(PowerSaver)
        );
    }

    #[test]
    fn ids_round_trip() {
        for mode in LowPowerMode::ALL {
            assert_eq!(
                LowPowerMode::from_id(&format!("{}\n", mode.id())),
                Some(mode)
            );
        }
        assert_eq!(LowPowerMode::from_id("sometimes"), None);
    }
}
