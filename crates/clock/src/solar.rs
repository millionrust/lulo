//! Where the sun is: the day/night map and each city's sunrise and sunset.
//!
//! Low-precision solar formulas (the Astronomical Almanac's, accurate to
//! about a minute), which is what a world clock needs.

use std::f64::consts::PI;

const J2000: f64 = 2_451_545.0;
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
/// Days from 1970-01-01 to 2000-01-01.
const J2000_UNIX_DAYS: i64 = 10_957;

fn julian_day(unix: f64) -> f64 {
    unix / 86_400.0 + UNIX_EPOCH_JD
}

fn unix_from_julian(julian: f64) -> f64 {
    (julian - UNIX_EPOCH_JD) * 86_400.0
}

/// The point on Earth with the sun overhead at `unix` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subsolar {
    /// Solar declination, radians.
    pub declination: f64,
    /// Longitude of the subsolar point, degrees east in −180…180.
    pub longitude: f64,
}

pub fn subsolar(unix: f64) -> Subsolar {
    let n = julian_day(unix) - J2000;
    let mean_longitude = (280.460 + 0.985_647_4 * n).rem_euclid(360.0);
    let anomaly = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let ecliptic =
        (mean_longitude + 1.915 * anomaly.sin() + 0.020 * (2.0 * anomaly).sin()).to_radians();
    let obliquity = (23.439 - 0.000_000_4 * n).to_radians();
    let declination = (obliquity.sin() * ecliptic.sin()).asin();
    let right_ascension = (obliquity.cos() * ecliptic.sin()).atan2(ecliptic.cos());
    let sidereal_hours = (18.697_374_558 + 24.065_709_824_419_08 * n).rem_euclid(24.0);
    let mut longitude = (right_ascension.to_degrees() - sidereal_hours * 15.0).rem_euclid(360.0);
    if longitude > 180.0 {
        longitude -= 360.0;
    }
    Subsolar {
        declination,
        longitude,
    }
}

impl Subsolar {
    /// Cosine of the sun's zenith angle at a place: positive means daylight.
    pub fn elevation_cosine(&self, latitude: f64, longitude: f64) -> f64 {
        let phi = latitude.to_radians();
        let hour = (longitude - self.longitude).to_radians();
        phi.sin() * self.declination.sin() + phi.cos() * self.declination.cos() * hour.cos()
    }

    pub fn is_day(&self, latitude: f64, longitude: f64) -> bool {
        self.elevation_cosine(latitude, longitude) > 0.0
    }
}

/// Sunrise and sunset on one civil date at a place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Daylight {
    /// Unix seconds of sunrise and sunset.
    Times { sunrise: i64, sunset: i64 },
    /// The sun never sets.
    PolarDay,
    /// The sun never rises.
    PolarNight,
}

/// The sunrise equation for the civil date `days` (days since 1970-01-01)
/// at `latitude`, `longitude` (degrees, east positive).
pub fn daylight(days: i64, latitude: f64, longitude: f64) -> Daylight {
    let n = (days - J2000_UNIX_DAYS) as f64;
    let mean_solar_noon = n - longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_solar_noon).rem_euclid(360.0);
    let m = anomaly.to_radians();
    let center = 1.9148 * m.sin() + 0.0200 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin();
    let ecliptic = (anomaly + center + 180.0 + 102.9372)
        .rem_euclid(360.0)
        .to_radians();
    let transit = J2000 + mean_solar_noon + 0.0053 * m.sin() - 0.0069 * (2.0 * ecliptic).sin();
    let declination = (ecliptic.sin() * 23.4397_f64.to_radians().sin()).asin();
    let phi = latitude.to_radians();
    let cos_hour = ((-0.833_f64).to_radians().sin() - phi.sin() * declination.sin())
        / (phi.cos() * declination.cos());
    if cos_hour < -1.0 {
        return Daylight::PolarDay;
    }
    if cos_hour > 1.0 {
        return Daylight::PolarNight;
    }
    let half_day = cos_hour.acos() / (2.0 * PI);
    Daylight::Times {
        sunrise: unix_from_julian(transit - half_day).round() as i64,
        sunset: unix_from_julian(transit + half_day).round() as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(days: i64, hours: f64) -> f64 {
        days as f64 * 86_400.0 + hours * 3600.0
    }

    #[test]
    fn equinox_sun_is_over_the_equator() {
        // 2026-09-23 is the September equinox; 20719 days after the epoch.
        let sun = subsolar(unix(20_719, 12.0));
        assert!(sun.declination.to_degrees().abs() < 0.5);
        // The sun crosses Greenwich about 7.5 min before 12:00 UTC in late
        // September (equation of time), so at noon it is about 2° west.
        assert!(
            sun.longitude < -1.0 && sun.longitude > -3.0,
            "{}",
            sun.longitude
        );
    }

    #[test]
    fn june_solstice_declination() {
        let sun = subsolar(unix(20_625, 12.0)); // 2026-06-21
        assert!((sun.declination.to_degrees() - 23.44).abs() < 0.1);
    }

    #[test]
    fn day_and_night_sides() {
        let sun = subsolar(unix(20_719, 6.75)); // 06:45 UTC = 12:15 IST
        assert!(sun.is_day(28.61, 77.21)); // New Delhi, noon
        assert!(!sun.is_day(40.71, -74.0)); // New York, 02:45
        assert!(sun.elevation_cosine(28.61, 77.21) > 0.8);
    }

    #[test]
    fn new_delhi_sunrise_matches_the_mac() {
        // The Mac's card read "Sunrise: 6:09 AM" on 2026-09-23 (IST, +5:30).
        let Daylight::Times { sunrise, sunset } = daylight(20_719, 28.61, 77.21) else {
            panic!("expected a sunrise");
        };
        let local = |time: i64| (time + 19_800).rem_euclid(86_400) / 60;
        assert!(
            (local(sunrise) - (6 * 60 + 9)).abs() <= 2,
            "{}",
            local(sunrise)
        );
        assert!(
            (local(sunset) - (18 * 60 + 17)).abs() <= 3,
            "{}",
            local(sunset)
        );
    }

    #[test]
    fn polar_day_and_night() {
        assert_eq!(daylight(20_625, 78.22, 15.65), Daylight::PolarDay); // Longyearbyen, June
        assert_eq!(daylight(20_806, 78.22, 15.65), Daylight::PolarNight); // December
    }
}
