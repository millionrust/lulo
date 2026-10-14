//! Weather ▸ Settings ▸ General ▸ Other Units: the wind, precipitation,
//! pressure and distance units the window shows, each either chosen or
//! following the region's defaults ("Restore Defaults"). Forecasts are
//! always fetched in metric units (km/h, mm, hPa, metres) and converted
//! here for display, so the cache never depends on a setting.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WindUnit {
    #[serde(rename = "km/h")]
    KilometresPerHour,
    #[serde(rename = "mph")]
    MilesPerHour,
    #[serde(rename = "m/s")]
    MetresPerSecond,
    #[serde(rename = "kn")]
    Knots,
    #[serde(rename = "Bft")]
    Beaufort,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PrecipitationUnit {
    #[serde(rename = "mm")]
    Millimetres,
    #[serde(rename = "cm")]
    Centimetres,
    #[serde(rename = "in")]
    Inches,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PressureUnit {
    #[serde(rename = "hPa")]
    Hectopascals,
    #[serde(rename = "kPa")]
    Kilopascals,
    #[serde(rename = "mb")]
    Millibars,
    #[serde(rename = "inHg")]
    InchesOfMercury,
    #[serde(rename = "mmHg")]
    MillimetresOfMercury,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DistanceUnit {
    #[serde(rename = "km")]
    Kilometres,
    #[serde(rename = "mi")]
    Miles,
}

impl WindUnit {
    pub const ALL: [Self; 5] = [
        Self::KilometresPerHour,
        Self::MilesPerHour,
        Self::MetresPerSecond,
        Self::Knots,
        Self::Beaufort,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::KilometresPerHour => "km/h",
            Self::MilesPerHour => "mph",
            Self::MetresPerSecond => "m/s",
            Self::Knots => "kn",
            Self::Beaufort => "Bft",
        }
    }

    /// `kilometres_per_hour` (Open-Meteo's `wind_speed_10m`) in this unit,
    /// rounded for the Wind tile: "11 km/h", "7 mph", "Force 2".
    pub fn format(self, kilometres_per_hour: f64) -> String {
        if !kilometres_per_hour.is_finite() {
            return "--".to_owned();
        }
        let value = match self {
            Self::KilometresPerHour => kilometres_per_hour,
            Self::MilesPerHour => kilometres_per_hour / 1.609_344,
            Self::MetresPerSecond => kilometres_per_hour / 3.6,
            Self::Knots => kilometres_per_hour / 1.852,
            Self::Beaufort => return format!("{} Bft", beaufort(kilometres_per_hour)),
        };
        format!("{:.0} {}", value.max(0.0), self.label())
    }
}

/// The Beaufort force for a wind speed: the WMO table's upper bounds in
/// km/h for forces 0 to 11; anything faster is 12.
pub fn beaufort(kilometres_per_hour: f64) -> u8 {
    const UPPER: [f64; 12] = [
        1.0, 5.0, 11.0, 19.0, 28.0, 38.0, 49.0, 61.0, 74.0, 88.0, 102.0, 117.0,
    ];
    UPPER
        .iter()
        .position(|&limit| kilometres_per_hour < limit)
        .unwrap_or(12) as u8
}

impl PrecipitationUnit {
    pub const ALL: [Self; 3] = [Self::Millimetres, Self::Centimetres, Self::Inches];

    pub fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "mm",
            Self::Centimetres => "cm",
            Self::Inches => "in",
        }
    }

    /// `millimetres` of rain (Open-Meteo's `precipitation_sum`) in this
    /// unit: "3 mm", "0.3 cm", "0.12 in".
    pub fn format(self, millimetres: f64) -> String {
        if !millimetres.is_finite() {
            return "--".to_owned();
        }
        let millimetres = millimetres.max(0.0);
        match self {
            Self::Millimetres => format!("{millimetres:.0} mm"),
            Self::Centimetres => format!("{:.1} cm", millimetres / 10.0),
            Self::Inches => format!("{:.2} in", millimetres / 25.4),
        }
    }
}

impl PressureUnit {
    pub const ALL: [Self; 5] = [
        Self::Hectopascals,
        Self::Kilopascals,
        Self::Millibars,
        Self::InchesOfMercury,
        Self::MillimetresOfMercury,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Hectopascals => "hPa",
            Self::Kilopascals => "kPa",
            Self::Millibars => "mb",
            Self::InchesOfMercury => "inHg",
            Self::MillimetresOfMercury => "mmHg",
        }
    }

    /// `hectopascals` (Open-Meteo's `pressure_msl`) in this unit.
    pub fn format(self, hectopascals: f64) -> String {
        if !hectopascals.is_finite() {
            return "--".to_owned();
        }
        match self {
            Self::Hectopascals => format!("{hectopascals:.0} hPa"),
            Self::Kilopascals => format!("{:.1} kPa", hectopascals / 10.0),
            Self::Millibars => format!("{hectopascals:.0} mb"),
            Self::InchesOfMercury => format!("{:.2} inHg", hectopascals * 0.029_529_983),
            Self::MillimetresOfMercury => format!("{:.0} mmHg", hectopascals * 0.750_061_68),
        }
    }
}

impl DistanceUnit {
    pub const ALL: [Self; 2] = [Self::Kilometres, Self::Miles];

    pub fn label(self) -> &'static str {
        match self {
            Self::Kilometres => "km",
            Self::Miles => "mi",
        }
    }

    /// `metres` (Open-Meteo's `visibility`) in this unit: "24 km", "15 mi".
    pub fn format(self, metres: f64) -> String {
        if !metres.is_finite() {
            return "--".to_owned();
        }
        let value = match self {
            Self::Kilometres => metres / 1000.0,
            Self::Miles => metres / 1609.344,
        };
        format!("{:.0} {}", value.max(0.0), self.label())
    }
}

/// The units a region uses when nothing is chosen ("Restore Defaults").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionUnits {
    pub wind: WindUnit,
    pub precipitation: PrecipitationUnit,
    pub pressure: PressureUnit,
    pub distance: DistanceUnit,
}

impl RegionUnits {
    /// The United States and its territories use imperial units, the UK
    /// keeps miles per hour and miles with metric rain and millibars, and
    /// everywhere else is metric — from the same `LC_MEASUREMENT`, `LC_ALL`
    /// or `LANG` region the temperature unit follows.
    pub fn for_locale(locale: &str) -> Self {
        let region = locale
            .split(['.', '@'])
            .next()
            .and_then(|tag| tag.split(['_', '-']).nth(1))
            .unwrap_or("");
        match region {
            "US" | "LR" | "MM" | "PR" | "GU" | "VI" | "AS" | "UM" => Self {
                wind: WindUnit::MilesPerHour,
                precipitation: PrecipitationUnit::Inches,
                pressure: PressureUnit::InchesOfMercury,
                distance: DistanceUnit::Miles,
            },
            "GB" => Self {
                wind: WindUnit::MilesPerHour,
                precipitation: PrecipitationUnit::Millimetres,
                pressure: PressureUnit::Millibars,
                distance: DistanceUnit::Miles,
            },
            _ => Self {
                wind: WindUnit::KilometresPerHour,
                precipitation: PrecipitationUnit::Millimetres,
                pressure: PressureUnit::Hectopascals,
                distance: DistanceUnit::Kilometres,
            },
        }
    }

    pub fn from_environment() -> Self {
        ["LC_ALL", "LC_MEASUREMENT", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .map_or_else(|| Self::for_locale(""), |locale| Self::for_locale(&locale))
    }
}

/// The Other Units choices as saved: `None` follows the region.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct UnitChoices {
    pub wind: Option<WindUnit>,
    pub precipitation: Option<PrecipitationUnit>,
    pub pressure: Option<PressureUnit>,
    pub distance: Option<DistanceUnit>,
}

impl UnitChoices {
    /// The units to show: each choice, or the region's default.
    pub fn resolve(self, region: RegionUnits) -> RegionUnits {
        RegionUnits {
            wind: self.wind.unwrap_or(region.wind),
            precipitation: self.precipitation.unwrap_or(region.precipitation),
            pressure: self.pressure.unwrap_or(region.pressure),
            distance: self.distance.unwrap_or(region.distance),
        }
    }

    pub fn is_default(self) -> bool {
        self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_converts_from_kilometres_per_hour() {
        assert_eq!(WindUnit::KilometresPerHour.format(11.2), "11 km/h");
        assert_eq!(WindUnit::MilesPerHour.format(16.09344), "10 mph");
        assert_eq!(WindUnit::MetresPerSecond.format(36.0), "10 m/s");
        assert_eq!(WindUnit::Knots.format(18.52), "10 kn");
        assert_eq!(WindUnit::Beaufort.format(11.2), "3 Bft");
        assert_eq!(WindUnit::KilometresPerHour.format(f64::NAN), "--");
    }

    #[test]
    fn beaufort_follows_the_wmo_table() {
        assert_eq!(beaufort(0.0), 0);
        assert_eq!(beaufort(4.9), 1);
        assert_eq!(beaufort(5.0), 2);
        assert_eq!(beaufort(62.0), 8);
        assert_eq!(beaufort(118.0), 12);
    }

    #[test]
    fn precipitation_pressure_and_distance_convert() {
        assert_eq!(PrecipitationUnit::Millimetres.format(3.4), "3 mm");
        assert_eq!(PrecipitationUnit::Centimetres.format(3.4), "0.3 cm");
        assert_eq!(PrecipitationUnit::Inches.format(25.4), "1.00 in");
        assert_eq!(PressureUnit::Hectopascals.format(1013.25), "1013 hPa");
        assert_eq!(PressureUnit::Kilopascals.format(1013.25), "101.3 kPa");
        assert_eq!(PressureUnit::Millibars.format(1013.25), "1013 mb");
        assert_eq!(PressureUnit::InchesOfMercury.format(1013.25), "29.92 inHg");
        assert_eq!(
            PressureUnit::MillimetresOfMercury.format(1013.25),
            "760 mmHg"
        );
        assert_eq!(DistanceUnit::Kilometres.format(24_140.0), "24 km");
        assert_eq!(DistanceUnit::Miles.format(24_140.16), "15 mi");
    }

    #[test]
    fn regions_pick_their_defaults_and_choices_override_them() {
        let us = RegionUnits::for_locale("en_US.UTF-8");
        assert_eq!(us.wind, WindUnit::MilesPerHour);
        assert_eq!(us.pressure, PressureUnit::InchesOfMercury);
        let india = RegionUnits::for_locale("en_IN.UTF-8");
        assert_eq!(india.wind, WindUnit::KilometresPerHour);
        assert_eq!(india.distance, DistanceUnit::Kilometres);
        let uk = RegionUnits::for_locale("en_GB.UTF-8");
        assert_eq!(uk.distance, DistanceUnit::Miles);
        assert_eq!(uk.precipitation, PrecipitationUnit::Millimetres);

        let choices = UnitChoices {
            wind: Some(WindUnit::Knots),
            ..UnitChoices::default()
        };
        assert!(!choices.is_default());
        let shown = choices.resolve(india);
        assert_eq!(shown.wind, WindUnit::Knots);
        assert_eq!(shown.pressure, PressureUnit::Hectopascals);
        assert!(UnitChoices::default().is_default());
    }

    #[test]
    fn choices_round_trip_with_the_unit_labels() {
        let choices = UnitChoices {
            wind: Some(WindUnit::MetresPerSecond),
            precipitation: None,
            pressure: Some(PressureUnit::MillimetresOfMercury),
            distance: Some(DistanceUnit::Miles),
        };
        let json = serde_json::to_string(&choices).unwrap();
        assert!(json.contains("\"m/s\"") && json.contains("\"mmHg\""));
        assert_eq!(serde_json::from_str::<UnitChoices>(&json).unwrap(), choices);
        assert_eq!(
            serde_json::from_str::<UnitChoices>("{}").unwrap(),
            UnitChoices::default()
        );
    }
}
