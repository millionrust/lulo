//! What the window shows, derived from a forecast: temperatures in the
//! chosen unit, the hourly strip with sunrise/sunset, the ten-day ranges and
//! the one-line outlook.

use crate::forecast::{Day, Family, Forecast, Sky};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unit {
    Celsius,
    Fahrenheit,
}

impl Unit {
    /// Fahrenheit for the United States and its territories, Celsius
    /// elsewhere — from `LC_MEASUREMENT`, `LC_ALL` or `LANG`.
    pub fn for_locale(locale: &str) -> Self {
        let region = locale
            .split(['.', '@'])
            .next()
            .and_then(|tag| tag.split(['_', '-']).nth(1))
            .unwrap_or("");
        if matches!(
            region,
            "US" | "LR" | "MM" | "PR" | "GU" | "VI" | "AS" | "UM"
        ) {
            Self::Fahrenheit
        } else {
            Self::Celsius
        }
    }

    pub fn from_environment() -> Self {
        ["LC_ALL", "LC_MEASUREMENT", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .map_or(Self::Celsius, |locale| Self::for_locale(&locale))
    }

    pub fn convert(self, celsius: f64) -> f64 {
        match self {
            Self::Celsius => celsius,
            Self::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
        }
    }

    /// "24°": whole degrees, no unit letter, as the Mac shows them.
    pub fn degrees(self, celsius: f64) -> String {
        let value = self.convert(celsius).round();
        // Never print "-0°".
        let value = if value == 0.0 { 0.0 } else { value };
        format!("{value:.0}°")
    }
}

/// Local wall-clock fields at a place.
fn local(time: i64, offset: i32) -> (i64, u32, u32) {
    let local = time + i64::from(offset);
    let seconds = local.rem_euclid(86_400);
    (
        local.div_euclid(86_400),
        (seconds / 3600) as u32,
        (seconds % 3600 / 60) as u32,
    )
}

/// "1PM", "12AM" — the hourly strip's compact hour.
pub fn hour_label(time: i64, offset: i32) -> String {
    let (_, hour, _) = local(time, offset);
    let (display, suffix) = twelve(hour);
    format!("{display}{suffix}")
}

/// "6:24PM" for sunrise and sunset in the strip; "6:24 PM" elsewhere.
pub fn time_label(time: i64, offset: i32, spaced: bool) -> String {
    let (_, hour, minute) = local(time, offset);
    let (display, suffix) = twelve(hour);
    let space = if spaced { " " } else { "" };
    format!("{display}:{minute:02}{space}{suffix}")
}

fn twelve(hour: u32) -> (u32, &'static str) {
    match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "Today" or the short weekday of a day's local midnight.
pub fn day_label(day_time: i64, now: i64, offset: i32) -> &'static str {
    let (day, _, _) = local(day_time + 43_200, offset);
    let (today, _, _) = local(now, offset);
    if day == today {
        "Today"
    } else {
        WEEKDAYS[(day + 3).rem_euclid(7) as usize]
    }
}

/// One column of the hourly strip.
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    Hour {
        label: String,
        sky: Sky,
        day: bool,
        temperature: f64,
        precipitation_chance: Option<f64>,
    },
    Sunrise {
        label: String,
    },
    Sunset {
        label: String,
    },
}

/// The next `count` hours from the current one ("Now" first), with sunrise
/// and sunset inserted where they fall.
pub fn hourly_strip(forecast: &Forecast, now: i64, count: usize) -> Vec<Column> {
    let offset = forecast.utc_offset;
    let start = forecast
        .hours
        .iter()
        .rposition(|hour| hour.time <= now)
        .unwrap_or(0);
    let hours = forecast
        .hours
        .iter()
        .skip(start)
        .take(count)
        .collect::<Vec<_>>();
    let Some(last) = hours.last().map(|hour| hour.time) else {
        return Vec::new();
    };
    let mut events = forecast
        .days
        .iter()
        .flat_map(|day| {
            [
                day.sunrise.map(|time| (time, true)),
                day.sunset.map(|time| (time, false)),
            ]
        })
        .flatten()
        .filter(|&(time, _)| time > now && time < last)
        .collect::<Vec<_>>();
    events.sort_unstable();
    let mut columns = Vec::with_capacity(hours.len() + events.len());
    let mut events = events.into_iter().peekable();
    for (index, hour) in hours.iter().enumerate() {
        while let Some(&(time, rising)) = events.peek() {
            if time >= hour.time {
                break;
            }
            let label = time_label(time, offset, false);
            columns.push(if rising {
                Column::Sunrise { label }
            } else {
                Column::Sunset { label }
            });
            events.next();
        }
        columns.push(Column::Hour {
            label: if index == 0 {
                "Now".to_owned()
            } else {
                hour_label(hour.time, offset)
            },
            sky: if index == 0 {
                forecast.current.sky
            } else {
                hour.sky
            },
            day: if index == 0 {
                forecast.current.day
            } else {
                hour.day
            },
            temperature: if index == 0 {
                forecast.current.temperature
            } else {
                hour.temperature
            },
            precipitation_chance: hour.precipitation_chance.filter(|chance| *chance >= 30.0),
        });
    }
    columns
}

/// The hourly card's sentence: the next change of conditions in the coming
/// twelve hours, or that the current ones continue.
pub fn outlook(forecast: &Forecast, now: i64) -> String {
    let family = forecast.current.sky.family();
    let change = forecast
        .hours
        .iter()
        .filter(|hour| hour.time > now && hour.time <= now + 12 * 3600)
        .find(|hour| hour.sky.family() != family);
    match change {
        Some(hour) => format!(
            "{} conditions expected around {}.",
            hour.sky.family().noun(),
            hour_label(hour.time, forecast.utc_offset)
        ),
        None => format!(
            "{} conditions will continue for the rest of the day.",
            family.noun()
        ),
    }
}

/// Where a day's low–high range sits on the shared ten-day scale, as
/// fractions 0…1 of the bar.
pub fn range_fractions(day: &Day, days: &[Day]) -> (f32, f32) {
    let low = days.iter().map(|day| day.low).fold(f64::INFINITY, f64::min);
    let high = days
        .iter()
        .map(|day| day.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (high - low).max(1.0);
    (
        ((day.low - low) / span).clamp(0.0, 1.0) as f32,
        ((day.high - low) / span).clamp(0.0, 1.0) as f32,
    )
}

/// Fraction along the ten-day scale for the current temperature (the dot
/// on today's bar).
pub fn current_fraction(temperature: f64, days: &[Day]) -> f32 {
    let low = days.iter().map(|day| day.low).fold(f64::INFINITY, f64::min);
    let high = days
        .iter()
        .map(|day| day.high)
        .fold(f64::NEG_INFINITY, f64::max);
    ((temperature - low) / (high - low).max(1.0)).clamp(0.0, 1.0) as f32
}

/// The layered glyph parts a Mac multicolour weather symbol is drawn from:
/// (asset name under icons/weather, colour 0xRRGGBB, x and y offset and
/// scale as fractions of the icon size).
pub fn sky_parts(sky: Sky, day: bool) -> &'static [(&'static str, u32, f32, f32, f32)] {
    const WHITE: u32 = 0xFFFFFF;
    const SUN: u32 = 0xFFD60A;
    const RAIN: u32 = 0x5AC8FA;
    match (sky, day) {
        (Sky::Clear, true) => &[("sun", SUN, 0.0, 0.0, 1.0)],
        (Sky::Clear, false) => &[("moon", WHITE, 0.0, 0.0, 1.0)],
        (Sky::MostlyClear | Sky::PartlyCloudy, true) => &[
            ("sun-small", SUN, 0.1, 0.02, 0.55),
            ("cloud-front", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::MostlyClear | Sky::PartlyCloudy, false) => &[
            ("moon-small", WHITE, 0.1, 0.02, 0.55),
            ("cloud-front", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Cloudy, _) => &[("cloud", WHITE, 0.0, 0.0, 1.0)],
        (Sky::Fog, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("fog", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Snow | Sky::HeavySnow | Sky::SnowShowers, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("snow", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Thunderstorms, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("bolt", SUN, 0.0, 0.0, 1.0),
        ],
        _ => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("rain", RAIN, 0.0, 0.0, 1.0),
        ],
    }
}

/// Background gradient (top, bottom) as 0xRRGGBB, by sky and daylight (S).
pub fn backdrop(sky: Sky, day: bool) -> (u32, u32) {
    match (sky.family(), day) {
        (Family::Clear, true) => (0x3D83D3, 0x74ABE6),
        (Family::Clear, false) => (0x0B1A3A, 0x2A3A60),
        (Family::Cloudy, true) => (0x5E7896, 0x93A6BD),
        (Family::Cloudy, false) => (0x252B37, 0x454C5A),
        (Family::Fog, true) => (0x8491A1, 0xB0B9C4),
        (Family::Fog, false) => (0x3A3F48, 0x5A606A),
        (Family::Rain, true) => (0x4B5563, 0x6F7887),
        (Family::Rain, false) => (0x1F242C, 0x3A414C),
        (Family::Snow, true) => (0x8A9BB0, 0xBBC6D3),
        (Family::Snow, false) => (0x2F3847, 0x535D6D),
        (Family::Storm, _) => (0x2B303B, 0x4A5263),
    }
}

/// Wind direction as a compass point.
pub fn compass(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    if !degrees.is_finite() {
        return "";
    }
    POINTS[((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

/// Wind speed in the unit's customary scale: km/h with Celsius, mph with
/// Fahrenheit.
pub fn wind(kilometres_per_hour: f64, unit: Unit) -> String {
    match unit {
        Unit::Celsius => format!("{:.0} km/h", kilometres_per_hour),
        Unit::Fahrenheit => format!("{:.0} mph", kilometres_per_hour / 1.609_344),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecast::tests::SAMPLE;

    fn sample() -> Forecast {
        Forecast::parse(SAMPLE.as_bytes()).unwrap()
    }

    #[test]
    fn units_follow_the_locale_and_round() {
        assert_eq!(Unit::for_locale("en_US.UTF-8"), Unit::Fahrenheit);
        assert_eq!(Unit::for_locale("en-US"), Unit::Fahrenheit);
        assert_eq!(Unit::for_locale("en_IN.UTF-8"), Unit::Celsius);
        assert_eq!(Unit::for_locale("hi_IN"), Unit::Celsius);
        assert_eq!(Unit::for_locale("C"), Unit::Celsius);
        assert_eq!(Unit::Celsius.degrees(24.4), "24°");
        assert_eq!(Unit::Celsius.degrees(-0.4), "0°");
        assert_eq!(Unit::Fahrenheit.degrees(24.4), "76°");
        assert_eq!(Unit::Fahrenheit.degrees(-40.0), "-40°");
    }

    #[test]
    fn labels_use_the_place_s_clock() {
        // 1790148600 is 13:00 in Bengaluru (+05:30).
        assert_eq!(hour_label(1_790_148_600, 19_800), "1PM");
        assert_eq!(hour_label(1_790_148_600 - 13 * 3600, 19_800), "12AM");
        assert_eq!(time_label(1_790_168_640, 19_800, false), "6:34PM");
        assert_eq!(time_label(1_790_168_640, 19_800, true), "6:34 PM");
        let now = 1_790_144_100;
        assert_eq!(day_label(1_790_101_800, now, 19_800), "Today");
        assert_eq!(day_label(1_790_188_200, now, 19_800), "Thu");
    }

    #[test]
    fn strip_starts_now_and_inserts_sun_events() {
        let mut forecast = sample();
        // Put a sunset between the 2nd and 3rd hours.
        forecast.days[0].sunset = Some(1_790_150_000);
        let strip = hourly_strip(&forecast, 1_790_144_100, 24);
        assert!(matches!(&strip[0], Column::Hour { label, temperature, .. }
            if label == "Now" && (*temperature - 24.4).abs() < 1e-9));
        assert!(matches!(&strip[1], Column::Hour { label, .. } if label == "12PM"));
        assert!(matches!(&strip[2], Column::Hour { label, .. } if label == "1PM"));
        assert!(matches!(&strip[3], Column::Sunset { label } if label == "1:23PM"));
        assert!(
            matches!(&strip[4], Column::Hour { precipitation_chance: Some(chance), .. }
            if *chance == 55.0)
        );
        assert_eq!(strip.len(), 5);
    }

    #[test]
    fn outlook_names_the_next_change() {
        let forecast = sample();
        assert_eq!(
            outlook(&forecast, 1_790_144_100),
            "Rainy conditions expected around 3PM."
        );
        let mut steady = forecast.clone();
        for hour in &mut steady.hours {
            hour.sky = Sky::PartlyCloudy;
        }
        assert_eq!(
            outlook(&steady, 1_790_144_100),
            "Cloudy conditions will continue for the rest of the day."
        );
    }

    #[test]
    fn ranges_share_one_scale() {
        let forecast = sample();
        let (low, high) = range_fractions(&forecast.days[0], &forecast.days);
        // Scale 18.9 … 27.2.
        assert!((low - 0.060).abs() < 0.01, "{low}");
        assert!((high - 1.0).abs() < 1e-6);
        let (low, _) = range_fractions(&forecast.days[1], &forecast.days);
        assert_eq!(low, 0.0);
        assert!((current_fraction(24.4, &forecast.days) - 0.663).abs() < 0.01);
    }

    #[test]
    fn compass_and_wind() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(250.0), "W");
        assert_eq!(compass(359.0), "N");
        assert_eq!(compass(f64::NAN), "");
        assert_eq!(wind(11.2, Unit::Celsius), "11 km/h");
        assert_eq!(wind(16.1, Unit::Fahrenheit), "10 mph");
    }
}
