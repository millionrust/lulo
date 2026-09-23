//! Open-Meteo forecast data: the request URL, the parsed response and the
//! WMO weather codes Weather understands.
//!
//! Source: <https://open-meteo.com/en/docs> — free, no API key, data under
//! CC BY 4.0 (the window credits "Weather data by Open-Meteo.com").

use serde::Deserialize;

pub const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
pub const FORECAST_DAYS: usize = 10;

/// The forecast request for one place. Temperatures are always fetched in
/// Celsius (converted for display) so the cache does not depend on units.
pub fn forecast_url(latitude: f64, longitude: f64) -> String {
    format!(
        "{FORECAST_ENDPOINT}?latitude={latitude:.4}&longitude={longitude:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,\
weather_code,wind_speed_10m,wind_direction_10m,precipitation\
         &hourly=temperature_2m,weather_code,is_day,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,\
uv_index_max,precipitation_probability_max\
         &timezone=auto&forecast_days={FORECAST_DAYS}&timeformat=unixtime"
    )
}

/// What the sky is doing, grouped the way the window draws it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sky {
    Clear,
    MostlyClear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    HeavyRain,
    FreezingRain,
    Snow,
    HeavySnow,
    Showers,
    SnowShowers,
    Thunderstorms,
}

impl Sky {
    /// WMO 4677 codes as Open-Meteo reports them; unknown codes are cloudy.
    pub fn from_wmo(code: u16) -> Self {
        match code {
            0 => Self::Clear,
            1 => Self::MostlyClear,
            2 => Self::PartlyCloudy,
            3 => Self::Cloudy,
            45 | 48 => Self::Fog,
            51 | 53 | 55 => Self::Drizzle,
            56 | 57 | 66 | 67 => Self::FreezingRain,
            61 | 63 => Self::Rain,
            65 => Self::HeavyRain,
            71 | 73 | 77 => Self::Snow,
            75 => Self::HeavySnow,
            80..=82 => Self::Showers,
            85 | 86 => Self::SnowShowers,
            95..=99 => Self::Thunderstorms,
            _ => Self::Cloudy,
        }
    }

    pub fn description(self, day: bool) -> &'static str {
        match self {
            Self::Clear if day => "Sunny",
            Self::Clear => "Clear",
            Self::MostlyClear if day => "Mostly Sunny",
            Self::MostlyClear => "Mostly Clear",
            Self::PartlyCloudy => "Partly Cloudy",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Foggy",
            Self::Drizzle => "Drizzle",
            Self::Rain => "Rain",
            Self::HeavyRain => "Heavy Rain",
            Self::FreezingRain => "Freezing Rain",
            Self::Snow => "Snow",
            Self::HeavySnow => "Heavy Snow",
            Self::Showers => "Showers",
            Self::SnowShowers => "Snow Showers",
            Self::Thunderstorms => "Thunderstorms",
        }
    }

    /// Broad family for backgrounds and "conditions expected" sentences.
    pub fn family(self) -> Family {
        match self {
            Self::Clear | Self::MostlyClear => Family::Clear,
            Self::PartlyCloudy | Self::Cloudy => Family::Cloudy,
            Self::Fog => Family::Fog,
            Self::Drizzle | Self::Rain | Self::HeavyRain | Self::FreezingRain | Self::Showers => {
                Family::Rain
            }
            Self::Snow | Self::HeavySnow | Self::SnowShowers => Family::Snow,
            Self::Thunderstorms => Family::Storm,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Family {
    Clear,
    Cloudy,
    Fog,
    Rain,
    Snow,
    Storm,
}

impl Family {
    pub fn noun(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Foggy",
            Self::Rain => "Rainy",
            Self::Snow => "Snowy",
            Self::Storm => "Stormy",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Current {
    pub time: i64,
    pub temperature: f64,
    pub feels_like: f64,
    pub humidity: f64,
    pub day: bool,
    pub sky: Sky,
    pub wind_speed: f64,
    pub wind_direction: f64,
    pub precipitation: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    pub time: i64,
    pub temperature: f64,
    pub sky: Sky,
    pub day: bool,
    pub precipitation_chance: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// Unix seconds of local midnight.
    pub time: i64,
    pub sky: Sky,
    pub high: f64,
    pub low: f64,
    pub sunrise: Option<i64>,
    pub sunset: Option<i64>,
    pub uv_index: Option<f64>,
    pub precipitation_chance: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    /// Seconds east of UTC at the place.
    pub utc_offset: i32,
    pub current: Current,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Json,
    Incomplete,
}

#[derive(Deserialize)]
struct RawForecast {
    utc_offset_seconds: i32,
    current: RawCurrent,
    hourly: RawHourly,
    daily: RawDaily,
}

#[derive(Deserialize)]
struct RawCurrent {
    time: i64,
    temperature_2m: f64,
    apparent_temperature: Option<f64>,
    relative_humidity_2m: Option<f64>,
    is_day: Option<u8>,
    weather_code: Option<u16>,
    wind_speed_10m: Option<f64>,
    wind_direction_10m: Option<f64>,
    precipitation: Option<f64>,
}

#[derive(Deserialize)]
struct RawHourly {
    time: Vec<i64>,
    temperature_2m: Vec<Option<f64>>,
    weather_code: Vec<Option<u16>>,
    is_day: Vec<Option<u8>>,
    #[serde(default)]
    precipitation_probability: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<i64>,
    weather_code: Vec<Option<u16>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    sunrise: Vec<Option<i64>>,
    #[serde(default)]
    sunset: Vec<Option<i64>>,
    #[serde(default)]
    uv_index_max: Vec<Option<f64>>,
    #[serde(default)]
    precipitation_probability_max: Vec<Option<f64>>,
}

fn at<T: Copy>(values: &[Option<T>], index: usize) -> Option<T> {
    values.get(index).copied().flatten()
}

impl Forecast {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let raw: RawForecast = serde_json::from_slice(bytes).map_err(|_| ParseError::Json)?;
        let current = Current {
            time: raw.current.time,
            temperature: raw.current.temperature_2m,
            feels_like: raw
                .current
                .apparent_temperature
                .unwrap_or(raw.current.temperature_2m),
            humidity: raw.current.relative_humidity_2m.unwrap_or(f64::NAN),
            day: raw.current.is_day.unwrap_or(1) != 0,
            sky: Sky::from_wmo(raw.current.weather_code.unwrap_or(3)),
            wind_speed: raw.current.wind_speed_10m.unwrap_or(f64::NAN),
            wind_direction: raw.current.wind_direction_10m.unwrap_or(f64::NAN),
            precipitation: raw.current.precipitation.unwrap_or(0.0),
        };
        let hourly = &raw.hourly;
        let hours = hourly
            .time
            .iter()
            .enumerate()
            .filter_map(|(index, &time)| {
                Some(Hour {
                    time,
                    temperature: at(&hourly.temperature_2m, index)?,
                    sky: Sky::from_wmo(at(&hourly.weather_code, index)?),
                    day: at(&hourly.is_day, index).unwrap_or(1) != 0,
                    precipitation_chance: at(&hourly.precipitation_probability, index),
                })
            })
            .collect::<Vec<_>>();
        let daily = &raw.daily;
        let days = daily
            .time
            .iter()
            .enumerate()
            .filter_map(|(index, &time)| {
                Some(Day {
                    time,
                    sky: Sky::from_wmo(at(&daily.weather_code, index)?),
                    high: at(&daily.temperature_2m_max, index)?,
                    low: at(&daily.temperature_2m_min, index)?,
                    sunrise: at(&daily.sunrise, index),
                    sunset: at(&daily.sunset, index),
                    uv_index: at(&daily.uv_index_max, index),
                    precipitation_chance: at(&daily.precipitation_probability_max, index),
                })
            })
            .take(FORECAST_DAYS)
            .collect::<Vec<_>>();
        if hours.is_empty() || days.is_empty() || !current.temperature.is_finite() {
            return Err(ParseError::Incomplete);
        }
        Ok(Self {
            utc_offset: raw.utc_offset_seconds,
            current,
            hours,
            days,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A trimmed real response shape (Bengaluru, 2026-09-23, +05:30).
    pub(crate) const SAMPLE: &str = r#"{
      "latitude": 12.98, "longitude": 77.58, "utc_offset_seconds": 19800,
      "timezone": "Asia/Kolkata",
      "current": {"time": 1790144100, "interval": 900, "temperature_2m": 24.4,
        "apparent_temperature": 25.9, "relative_humidity_2m": 71, "is_day": 1,
        "weather_code": 2, "wind_speed_10m": 11.2, "wind_direction_10m": 250,
        "precipitation": 0.0},
      "hourly": {
        "time": [1790141400, 1790145000, 1790148600, 1790152200, 1790155800],
        "temperature_2m": [24.1, 25.0, 25.8, null, 24.3],
        "weather_code": [2, 2, 3, 61, 80],
        "is_day": [1, 1, 1, 1, 1],
        "precipitation_probability": [5, 10, 35, 60, 55]
      },
      "daily": {
        "time": [1790101800, 1790188200],
        "weather_code": [2, 95],
        "temperature_2m_max": [27.2, 26.1],
        "temperature_2m_min": [19.4, 18.9],
        "sunrise": [1790124840, 1790211240],
        "sunset": [1790168640, 1790255000],
        "uv_index_max": [7.1, 6.5],
        "precipitation_probability_max": [40, 80]
      }
    }"#;

    #[test]
    fn url_asks_for_everything_the_window_shows() {
        let url = forecast_url(12.9716, 77.5946);
        assert!(url.starts_with(
            "https://api.open-meteo.com/v1/forecast?latitude=12.9716&longitude=77.5946&"
        ));
        for part in [
            "current=temperature_2m,apparent_temperature",
            "hourly=temperature_2m,weather_code,is_day,precipitation_probability",
            "daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,",
            "forecast_days=10",
            "timeformat=unixtime",
            "timezone=auto",
        ] {
            assert!(url.contains(part), "{part}");
        }
        assert!(!url.contains(' '));
    }

    #[test]
    fn parses_the_sample() {
        let forecast = Forecast::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(forecast.utc_offset, 19_800);
        assert_eq!(forecast.current.sky, Sky::PartlyCloudy);
        assert!((forecast.current.feels_like - 25.9).abs() < 1e-9);
        // The hour with a null temperature is skipped, not zeroed.
        assert_eq!(forecast.hours.len(), 4);
        assert_eq!(forecast.hours[3].sky, Sky::Showers);
        assert_eq!(forecast.days.len(), 2);
        assert_eq!(forecast.days[1].sky, Sky::Thunderstorms);
        assert_eq!(forecast.days[0].sunset, Some(1_790_168_640));
    }

    #[test]
    fn rejects_garbage_and_empty_forecasts() {
        assert_eq!(Forecast::parse(b"<html>"), Err(ParseError::Json));
        let empty = SAMPLE.replace("\"time\": [1790101800, 1790188200]", "\"time\": []");
        assert_eq!(
            Forecast::parse(empty.as_bytes()),
            Err(ParseError::Incomplete)
        );
    }

    #[test]
    fn wmo_codes_map_to_skies() {
        assert_eq!(Sky::from_wmo(0).description(true), "Sunny");
        assert_eq!(Sky::from_wmo(0).description(false), "Clear");
        assert_eq!(Sky::from_wmo(1).description(true), "Mostly Sunny");
        assert_eq!(Sky::from_wmo(48), Sky::Fog);
        assert_eq!(Sky::from_wmo(57), Sky::FreezingRain);
        assert_eq!(Sky::from_wmo(65).family(), Family::Rain);
        assert_eq!(Sky::from_wmo(86).family(), Family::Snow);
        assert_eq!(Sky::from_wmo(99), Sky::Thunderstorms);
        assert_eq!(Sky::from_wmo(1234), Sky::Cloudy);
    }
}
