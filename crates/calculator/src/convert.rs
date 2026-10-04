//! The Convert-mode state: pure logic, no GPUI (CALC-06, View ▸ Convert
//! ⌥⌘C).
//!
//! This reuses Spotlight's own unit-conversion engine
//! (`rmac_launcher_providers::conversion`) rather than a second copy of the
//! unit tables and factor maths: the same [`Unit`] rows and the same
//! `value × factor + offset` formula back both the Spotlight answer card
//! and this mode. Currency is deliberately not included here: Spotlight's
//! currency provider fetches live ECB rates over the network and is gated
//! behind its own Settings ▸ Spotlight network permission
//! (`rmac_launcher_providers::currency`), and Calculator has no equivalent
//! settings surface to host that consent — out of scope for this pass.

use rmac_launcher_providers::conversion::{Dimension, Unit, TEMPERATURES, UNITS};

/// The categories shown in Convert's category selector, in a reasonable
/// (not Mac-measured) order: most everyday dimensions first.
pub const CATEGORIES: [Dimension; 8] = [
    Dimension::Length,
    Dimension::Mass,
    Dimension::Temperature,
    Dimension::Volume,
    Dimension::Speed,
    Dimension::Area,
    Dimension::Time,
    Dimension::Data,
];

pub fn category_label(dimension: Dimension) -> &'static str {
    match dimension {
        Dimension::Length => "Length",
        Dimension::Mass => "Weight and Mass",
        Dimension::Temperature => "Temperature",
        Dimension::Volume => "Volume",
        Dimension::Speed => "Speed",
        Dimension::Area => "Area",
        Dimension::Time => "Time",
        Dimension::Data => "Data",
    }
}

/// Every unit in `dimension`, in the fixed table order.
pub fn units_for(dimension: Dimension) -> Vec<&'static Unit> {
    if dimension == Dimension::Temperature {
        TEMPERATURES.iter().collect()
    } else {
        UNITS
            .iter()
            .filter(|unit| unit.dimension == dimension)
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct ConvertState {
    category: usize,
    from_unit: usize,
    to_unit: usize,
    /// Digits typed for the "from" amount, as plain text (e.g. "12.5").
    entry: String,
}

impl Default for ConvertState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConvertState {
    pub fn new() -> Self {
        Self {
            category: 0,
            from_unit: 0,
            to_unit: 1.min(units_for(CATEGORIES[0]).len().saturating_sub(1)),
            entry: String::new(),
        }
    }

    pub fn category(&self) -> Dimension {
        CATEGORIES[self.category]
    }

    fn units(&self) -> Vec<&'static Unit> {
        units_for(self.category())
    }

    pub fn from_unit(&self) -> &'static Unit {
        let units = self.units();
        units[self.from_unit.min(units.len() - 1)]
    }

    pub fn to_unit(&self) -> &'static Unit {
        let units = self.units();
        units[self.to_unit.min(units.len() - 1)]
    }

    pub fn next_category(&mut self) {
        self.category = (self.category + 1) % CATEGORIES.len();
        self.from_unit = 0;
        self.to_unit = 1.min(self.units().len().saturating_sub(1));
        self.entry.clear();
    }

    pub fn next_from_unit(&mut self) {
        let len = self.units().len();
        if len > 0 {
            self.from_unit = (self.from_unit + 1) % len;
        }
    }

    pub fn next_to_unit(&mut self) {
        let len = self.units().len();
        if len > 0 {
            self.to_unit = (self.to_unit + 1) % len;
        }
    }

    /// Swap the two units, like the Mac's ⇄ button.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.from_unit, &mut self.to_unit);
    }

    pub fn digit(&mut self, digit: u8) {
        if digit > 9 || self.entry.len() >= 15 {
            return;
        }
        self.entry.push((b'0' + digit) as char);
    }

    pub fn decimal(&mut self) {
        if !self.entry.contains('.') {
            if self.entry.is_empty() {
                self.entry.push('0');
            }
            self.entry.push('.');
        }
    }

    pub fn backspace(&mut self) {
        self.entry.pop();
    }

    pub fn clear(&mut self) {
        self.entry.clear();
    }

    /// The typed "from" amount, `0` when nothing has been typed.
    pub fn amount(&self) -> f64 {
        self.entry.parse().unwrap_or(0.0)
    }

    pub fn from_text(&self) -> String {
        if self.entry.is_empty() {
            "0".to_owned()
        } else {
            self.entry.clone()
        }
    }

    /// The converted "to" amount, formatted with a sensible number of
    /// decimals (trailing zeroes trimmed, whole numbers bare).
    pub fn to_text(&self) -> String {
        format_amount(self.converted())
    }

    /// `amount` converted from `from_unit` to `to_unit` via the shared
    /// `value × factor + offset` base-unit formula (the same arithmetic
    /// `rmac_launcher_providers::conversion::convert` uses internally).
    pub fn converted(&self) -> f64 {
        let from = self.from_unit();
        let to = self.to_unit();
        let base = self.amount() * from.factor + from.offset;
        (base - to.offset) / to.factor
    }
}

fn format_amount(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let fixed = format!("{value:.6}");
    let trimmed = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        &fixed
    };
    let trimmed = if trimmed.is_empty() || trimmed == "-0" {
        "0"
    } else {
        trimmed
    };
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_kilometres_to_miles_using_the_shared_unit_table() {
        let mut state = ConvertState::new();
        // Length is category 0; put km first, miles second.
        let km = state.units().iter().position(|u| u.symbol == "km").unwrap();
        let mi = state.units().iter().position(|u| u.symbol == "mi").unwrap();
        state.from_unit = km;
        state.to_unit = mi;
        state.digit(5);
        assert_eq!(state.from_text(), "5");
        // 5 km ≈ 3.106855 mi.
        let converted = state.converted();
        assert!((converted - 3.106_855).abs() < 1e-3, "{converted}");
    }

    #[test]
    fn temperature_uses_the_offset_table() {
        let mut state = ConvertState::new();
        while state.category() != Dimension::Temperature {
            state.next_category();
        }
        let c = state.units().iter().position(|u| u.symbol == "°C").unwrap();
        let f = state.units().iter().position(|u| u.symbol == "°F").unwrap();
        state.from_unit = c;
        state.to_unit = f;
        state.digit(0);
        // 0°C = 32°F.
        assert_eq!(state.to_text(), "32");
    }

    #[test]
    fn swap_exchanges_the_two_units() {
        let mut state = ConvertState::new();
        let (from, to) = (state.from_unit, state.to_unit);
        state.swap();
        assert_eq!(state.from_unit, to);
        assert_eq!(state.to_unit, from);
    }

    #[test]
    fn category_change_resets_entry_and_units() {
        let mut state = ConvertState::new();
        state.digit(7);
        state.next_category();
        assert_eq!(state.from_text(), "0");
        assert_eq!(state.category(), CATEGORIES[1]);
    }

    #[test]
    fn decimal_point_only_appears_once() {
        let mut state = ConvertState::new();
        state.digit(1);
        state.decimal();
        state.digit(5);
        state.decimal();
        assert_eq!(state.from_text(), "1.5");
    }
}
