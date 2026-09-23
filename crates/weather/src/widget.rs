//! What the Weather desktop and Notification Centre widget shows: the
//! selected place's current conditions and today's range, read from the
//! forecast cache the app keeps, refreshed through the same request when
//! the cache is stale. No window code, so the shell can use it without
//! the app's UI dependencies.

use crate::fetch::{self, FetchError};
use crate::forecast::{self, Forecast, Sky};
use crate::geocode::Place;
use crate::store::{self, Cached};
use crate::summary::{self, Unit};

/// One place's widget content, already formatted in the chosen unit.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetWeather {
    pub place: String,
    pub temperature: String,
    pub description: &'static str,
    /// "H:27° L:19°", empty when the forecast has no day for today.
    pub range: String,
    pub sky: Sky,
    pub day: bool,
    /// Background gradient (top, bottom) as 0xRRGGBB.
    pub backdrop: (u32, u32),
    pub fetched_at: i64,
}

/// Why there is nothing to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unavailable {
    /// No place has been added in Weather yet.
    NoPlace,
    /// A place exists but no forecast could be read or fetched.
    NoForecast,
}

pub fn from_forecast(
    place: &Place,
    forecast: &Forecast,
    fetched_at: i64,
    now: i64,
    unit: Unit,
) -> WidgetWeather {
    let current = &forecast.current;
    let today = forecast
        .days
        .iter()
        .find(|day| now >= day.time && now < day.time + 86_400)
        .or_else(|| forecast.days.first());
    WidgetWeather {
        place: place.name.clone(),
        temperature: unit.degrees(current.temperature),
        description: current.sky.description(current.day),
        range: today
            .map(|day| format!("H:{} L:{}", unit.degrees(day.high), unit.degrees(day.low)))
            .unwrap_or_default(),
        sky: current.sky,
        day: current.day,
        backdrop: summary::backdrop(current.sky, current.day),
        fetched_at,
    }
}

/// The selected place's widget content. With `refresh`, a cache older
/// than [`store::FRESH_SECONDS`] is fetched again (blocking, through curl)
/// and written back so the app sees it too.
pub fn load(now: i64, refresh: bool) -> Result<WidgetWeather, Unavailable> {
    let settings = store::load_settings().map_err(|_| Unavailable::NoPlace)?;
    let place = settings.current().cloned().ok_or(Unavailable::NoPlace)?;
    let unit = settings
        .fahrenheit
        .map(|fahrenheit| {
            if fahrenheit {
                Unit::Fahrenheit
            } else {
                Unit::Celsius
            }
        })
        .unwrap_or_else(Unit::from_environment);
    let path = store::cache_path(&place);
    let cached = path
        .as_deref()
        .and_then(store::read_cache_from)
        .and_then(|cached| {
            Forecast::parse(cached.body.as_bytes())
                .ok()
                .map(|forecast| (forecast, cached.fetched_at))
        });
    let stale = cached
        .as_ref()
        .is_none_or(|(_, fetched_at)| now - fetched_at >= store::FRESH_SECONDS);
    if refresh && stale {
        if let Ok((forecast, body)) = fetch_forecast(&place) {
            if let Some(path) = path.as_deref() {
                let _ = store::write_cache_to(
                    path,
                    &Cached {
                        fetched_at: now,
                        body,
                    },
                );
            }
            return Ok(from_forecast(&place, &forecast, now, now, unit));
        }
    }
    cached
        .map(|(forecast, fetched_at)| from_forecast(&place, &forecast, fetched_at, now, unit))
        .ok_or(Unavailable::NoForecast)
}

fn fetch_forecast(place: &Place) -> Result<(Forecast, String), FetchError> {
    let body = fetch::get(&forecast::forecast_url(place.latitude, place.longitude))?;
    let forecast = Forecast::parse(&body).map_err(|_| FetchError::Service)?;
    let body = String::from_utf8(body).map_err(|_| FetchError::Service)?;
    Ok((forecast, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecast::tests::SAMPLE;

    #[test]
    fn widget_shows_current_conditions_and_today_s_range() {
        let forecast = Forecast::parse(SAMPLE.as_bytes()).unwrap();
        let place = Place {
            name: "Bengaluru".to_owned(),
            region: String::new(),
            latitude: 12.97,
            longitude: 77.59,
        };
        let now = forecast.current.time;
        let widget = from_forecast(&place, &forecast, now - 60, now, Unit::Celsius);
        assert_eq!(widget.place, "Bengaluru");
        assert_eq!(
            widget.temperature,
            Unit::Celsius.degrees(forecast.current.temperature)
        );
        assert_eq!(
            widget.description,
            forecast.current.sky.description(forecast.current.day)
        );
        let today = forecast
            .days
            .iter()
            .find(|day| now >= day.time && now < day.time + 86_400)
            .unwrap_or(&forecast.days[0]);
        assert_eq!(
            widget.range,
            format!(
                "H:{} L:{}",
                Unit::Celsius.degrees(today.high),
                Unit::Celsius.degrees(today.low)
            )
        );
        assert_eq!(
            widget.backdrop,
            summary::backdrop(forecast.current.sky, forecast.current.day)
        );
    }
}
