//! "time in tokyo": the answer card with a city's local time.
//!
//! Measured on macOS 26.2: "Tokyo, Japan" · "GMT+9 · Tomorrow, +3:30 HRS"
//! · "12:46 AM", and the completion plate "— Tokyo, Japan". The wording
//! matches rmac Clock's World Clock, which owns the city list and the time
//! zone database; the launcher reaches them through [`CityClock`].

use super::*;
use crate::locale::time_12h;

/// One city's zone at an instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CityTime {
    pub city: String,
    pub country: String,
    /// IANA name, e.g. `Asia/Tokyo`.
    pub zone: String,
    /// Seconds east of UTC in the city and here, at `utc`.
    pub offset: i32,
    pub local_offset: i32,
    /// Unix seconds.
    pub utc: i64,
}

/// Cities by name, best match first, with their offsets now.
pub trait CityClock: Send + Sync + 'static {
    fn cities(&self, name: &str) -> Vec<CityTime>;
}

pub struct WorldClockProvider<C> {
    clock: C,
}

impl<C> WorldClockProvider<C> {
    pub fn new(clock: C) -> Self {
        Self { clock }
    }
}

impl<C: CityClock> Provider for WorldClockProvider<C> {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(WORLD_CLOCK_PROVIDER, Category::Clock, Privacy::default())
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let Some(place) = place_in_time_query(query) else {
            return Ok(Vec::new());
        };
        Ok(self
            .clock
            .cities(&place)
            .into_iter()
            .next()
            .map(|city| city_result(&city))
            .into_iter()
            .collect())
    }
}

/// The place in "time in tokyo", "what time is it in new york",
/// "current time in paris", "tokyo time" or "time tokyo".
pub fn place_in_time_query(query: &str) -> Option<String> {
    let query = query
        .trim()
        .trim_end_matches('?')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let place = [
        "what time is it in ",
        "what's the time in ",
        "whats the time in ",
        "current time in ",
        "local time in ",
        "time now in ",
        "time in ",
        "time at ",
        "time ",
    ]
    .iter()
    .find_map(|prefix| query.strip_prefix(prefix))
    .or_else(|| {
        query
            .strip_suffix(" local time")
            .or_else(|| query.strip_suffix(" time now"))
            .or_else(|| query.strip_suffix(" time"))
    })?
    .trim();
    (place.chars().count() >= 2).then(|| place.to_owned())
}

/// The answer card: title "Tokyo, Japan", subtitle "GMT+9 · Tomorrow,
/// +3:30 HRS", detail "12:46 AM". Return copies the time.
pub fn city_result(city: &CityTime) -> SearchResult {
    let local = wall(city.utc, city.local_offset);
    let there = wall(city.utc, city.offset);
    let time = time_12h(there.1, there.2);
    SearchResult {
        id: ResultId {
            provider: provider_id(WORLD_CLOCK_PROVIDER),
            local: format!("{}|{}", city.zone, city.city),
        },
        category: Category::Clock,
        application_group: None,
        title: format!("{}, {}", city.city, city.country),
        subtitle: Some(format!(
            "{} · {}, {}",
            gmt_offset(city.offset),
            relative_day(there.0, local.0),
            offset_difference(city.offset, city.local_offset)
        )),
        detail: Some(time.clone()),
        icon: None,
        primary: Action::CopyText { text: time },
        alternate: None,
        recency_rank: 0,
    }
}

/// (days since 1970, hour, minute) at `offset`.
fn wall(utc: i64, offset: i32) -> (i64, u32, u32) {
    let local = utc + i64::from(offset);
    let seconds = local.rem_euclid(86_400);
    (
        local.div_euclid(86_400),
        (seconds / 3_600) as u32,
        (seconds % 3_600 / 60) as u32,
    )
}

/// "GMT+9", "GMT+5:30", "GMT-3", "GMT".
pub fn gmt_offset(offset: i32) -> String {
    if offset == 0 {
        return "GMT".into();
    }
    let sign = if offset < 0 { '-' } else { '+' };
    let magnitude = offset.unsigned_abs();
    let (hours, minutes) = (magnitude / 3_600, magnitude % 3_600 / 60);
    if minutes == 0 {
        format!("GMT{sign}{hours}")
    } else {
        format!("GMT{sign}{hours}:{minutes:02}")
    }
}

/// "Today", "Yesterday" or "Tomorrow" for the city relative to here.
pub fn relative_day(city_days: i64, local_days: i64) -> &'static str {
    match city_days - local_days {
        ..=-1 => "Yesterday",
        0 => "Today",
        _ => "Tomorrow",
    }
}

/// "+0 HRS", "+1 HR", "-4 HRS", "+3:30 HRS": the city's offset from here.
pub fn offset_difference(city_offset: i32, local_offset: i32) -> String {
    let difference = city_offset - local_offset;
    let sign = if difference < 0 { '-' } else { '+' };
    let magnitude = difference.unsigned_abs();
    let hours = magnitude / 3_600;
    let minutes = magnitude % 3_600 / 60;
    let unit = if hours == 1 && minutes == 0 {
        "HR"
    } else {
        "HRS"
    };
    if minutes == 0 {
        format!("{sign}{hours} {unit}")
    } else {
        format!("{sign}{hours}:{minutes:02} {unit}")
    }
}
