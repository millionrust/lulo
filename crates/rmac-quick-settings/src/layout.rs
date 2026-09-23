//! Control Center module grid and the surface size that fits it exactly.
//!
//! Measured from macOS 26 on the owner's Mac (Retina captures, see
//! `design-lab/control-center.html`): modules sit on a four-column grid of
//! 64 pt cells with a 12 pt gap both ways. rmac's layer surface adds its own
//! padding around the grid, and its height follows the modules shown so the
//! compositor's rounded blur and shadow cover the modules and nothing else.

use crate::{Command, PowerValue};

/// Side of one grid cell: a circle module, or the height of a pill or row.
pub const CELL: f64 = 64.0;
/// Gap between modules, horizontally and vertically.
pub const GAP: f64 = 12.0;
pub const COLUMNS: usize = 4;
/// Four cells and three gaps: 292.
pub const GRID_WIDTH: f64 = CELL * COLUMNS as f64 + GAP * (COLUMNS - 1) as f64;
/// A two-cell pill (Wi-Fi, Bluetooth, Focus): 140.
pub const PILL_WIDTH: f64 = CELL * 2.0 + GAP;
/// Pills and circles are fully rounded.
pub const PILL_RADIUS: f64 = CELL / 2.0;
/// Now Playing and the full-width slider modules.
pub const MODULE_RADIUS: f64 = 26.0;
/// rmac-only: padding between the surface edge and the grid. There is no
/// hard-edged panel on the Mac; this equals the module gap.
pub const PADDING: f64 = GAP;
/// rmac-only: concentric with the pills inside the padding.
pub const SURFACE_RADIUS: f64 = PILL_RADIUS + PADDING;
pub const SURFACE_WIDTH: f64 = GRID_WIDTH + PADDING * 2.0;
/// One error banner above the grid; banners past the limit are not shown.
pub const BANNER_HEIGHT: f64 = 40.0;
pub const MAX_BANNERS: usize = 3;

/// Which optional modules the panel shows right now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modules {
    /// An MPRIS player is active: Now Playing takes the right half of the
    /// first two rows and Bluetooth drops below Wi-Fi.
    pub now_playing: bool,
    /// A backlight exists, so the Display brightness module is shown.
    pub display: bool,
    /// Error banners above the grid, clamped to [`MAX_BANNERS`].
    pub banners: usize,
}

impl Modules {
    /// Grid rows: Wi-Fi/Bluetooth (one row, or two beside Now Playing), the
    /// Low Power, Screenshot and Focus row, Display when present, and Sound.
    pub const fn rows(self) -> usize {
        let pills = if self.now_playing { 2 } else { 1 };
        let display = if self.display { 1 } else { 0 };
        pills + 1 + display + 1
    }

    pub const fn banners_shown(self) -> usize {
        if self.banners > MAX_BANNERS {
            MAX_BANNERS
        } else {
            self.banners
        }
    }

    /// Top of grid row `row` inside the content box, below any banners.
    pub const fn row_top(self, row: usize) -> f64 {
        self.banners_shown() as f64 * (BANNER_HEIGHT + GAP) + row as f64 * (CELL + GAP)
    }

    /// Height of the banners and module rows, without the surface padding.
    pub const fn content_height(self) -> f64 {
        let rows = self.rows() as f64;
        self.banners_shown() as f64 * (BANNER_HEIGHT + GAP) + rows * CELL + (rows - 1.0) * GAP
    }

    /// Logical height of the layer surface that fits the modules exactly.
    pub const fn surface_height(self) -> f64 {
        self.content_height() + PADDING * 2.0
    }
}

/// The surface before anything optional is known: 240.
pub const MIN_SURFACE_HEIGHT: f64 = Modules {
    now_playing: false,
    display: false,
    banners: 0,
}
.surface_height();

/// Every optional module and the most banners: the size an output must fit.
pub const MAX_SURFACE_HEIGHT: f64 = Modules {
    now_playing: true,
    display: true,
    banners: MAX_BANNERS,
}
.surface_height();

/// Low Power Mode is shown only when power-profiles-daemon offers
/// power-saver and something to return to.
pub fn low_power_available(power: &PowerValue) -> bool {
    power
        .supported
        .contains(&rmac_power::PowerProfile::PowerSaver)
        && power
            .supported
            .iter()
            .any(|profile| *profile != rmac_power::PowerProfile::PowerSaver)
}

pub fn low_power_enabled(power: &PowerValue) -> bool {
    power.active == Some(rmac_power::PowerProfile::PowerSaver)
}

/// The macOS Low Power Mode control: switch to power-saver, or back to
/// balanced (the "Automatic" profile) when it is already on.
pub fn low_power_toggle(power: &PowerValue) -> Option<Command> {
    use rmac_power::PowerProfile;
    if !low_power_available(power) {
        return None;
    }
    let target = if low_power_enabled(power) {
        if power.supported.contains(&PowerProfile::Balanced) {
            PowerProfile::Balanced
        } else {
            *power
                .supported
                .iter()
                .find(|profile| **profile != PowerProfile::PowerSaver)?
        }
    } else {
        PowerProfile::PowerSaver
    };
    Some(Command::SetPowerProfile(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_power::PowerProfile;

    #[test]
    fn grid_matches_the_measured_mac_geometry() {
        assert_eq!(GRID_WIDTH, 292.0);
        assert_eq!(PILL_WIDTH, 140.0);
        assert_eq!(SURFACE_WIDTH, 316.0);
        assert_eq!(SURFACE_RADIUS, 44.0);
    }

    #[test]
    fn surface_height_fits_the_rows_exactly() {
        // Wi-Fi|Bluetooth, circles|Focus, Sound.
        assert_eq!(Modules::default().rows(), 3);
        assert_eq!(MIN_SURFACE_HEIGHT, 12.0 + 3.0 * 64.0 + 2.0 * 12.0 + 12.0);
        // Now Playing adds a row; Display adds a row.
        let full = Modules {
            now_playing: true,
            display: true,
            banners: 0,
        };
        assert_eq!(full.rows(), 5);
        assert_eq!(full.content_height(), 5.0 * 64.0 + 4.0 * 12.0);
        assert_eq!(full.surface_height(), 368.0 + 24.0);
        assert_eq!(full.row_top(4), 304.0);
    }

    #[test]
    fn banners_push_the_grid_down_and_are_clamped() {
        let one = Modules {
            banners: 1,
            ..Modules::default()
        };
        assert_eq!(one.row_top(0), 52.0);
        assert_eq!(
            one.surface_height(),
            MIN_SURFACE_HEIGHT + BANNER_HEIGHT + GAP
        );
        let many = Modules {
            banners: 9,
            ..Modules::default()
        };
        assert_eq!(many.banners_shown(), MAX_BANNERS);
        assert!(MAX_SURFACE_HEIGHT >= many.surface_height());
    }

    fn power(active: Option<PowerProfile>, supported: &[PowerProfile]) -> PowerValue {
        PowerValue {
            active,
            supported: supported.to_vec(),
        }
    }

    #[test]
    fn low_power_toggles_between_saver_and_balanced() {
        let all = [
            PowerProfile::PowerSaver,
            PowerProfile::Balanced,
            PowerProfile::Performance,
        ];
        assert_eq!(
            low_power_toggle(&power(Some(PowerProfile::Balanced), &all)),
            Some(Command::SetPowerProfile(PowerProfile::PowerSaver))
        );
        assert_eq!(
            low_power_toggle(&power(Some(PowerProfile::Performance), &all)),
            Some(Command::SetPowerProfile(PowerProfile::PowerSaver))
        );
        assert_eq!(
            low_power_toggle(&power(Some(PowerProfile::PowerSaver), &all)),
            Some(Command::SetPowerProfile(PowerProfile::Balanced))
        );
        let no_balanced = [PowerProfile::PowerSaver, PowerProfile::Performance];
        assert_eq!(
            low_power_toggle(&power(Some(PowerProfile::PowerSaver), &no_balanced)),
            Some(Command::SetPowerProfile(PowerProfile::Performance))
        );
    }

    #[test]
    fn low_power_is_hidden_without_a_saver_profile() {
        let value = power(
            Some(PowerProfile::Balanced),
            &[PowerProfile::Balanced, PowerProfile::Performance],
        );
        assert!(!low_power_available(&value));
        assert_eq!(low_power_toggle(&value), None);
        assert!(!low_power_available(&power(
            None,
            &[PowerProfile::PowerSaver]
        )));
    }
}
