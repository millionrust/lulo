//! "time in tokyo" answered from rmac Clock's city list and the system time
//! zone database, so Spotlight and the World Clock agree.

use rmac_launcher_providers::{CityClock, CityTime};

/// Cities matching a name, best first (rmac Clock's own ordering: city-name
/// prefixes, then other word or country matches).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemClock;

/// A handful is enough: the card shows the first.
const MAX_CITIES: usize = 3;

impl CityClock for SystemClock {
    fn cities(&self, name: &str) -> Vec<CityTime> {
        let utc = rmac_launcher_providers::locale::unix_now();
        let local = rmac_clock::tz::local_zone();
        let local_offset = local.offset_at(utc);
        rmac_clock::cities::search(name)
            .into_iter()
            .take(MAX_CITIES)
            .filter_map(|city| {
                let zone = rmac_clock::tz::Zone::load(city.zone).ok()?;
                Some(CityTime {
                    city: city.name.to_owned(),
                    country: city.country.to_owned(),
                    zone: city.zone.to_owned(),
                    offset: zone.offset_at(utc),
                    local_offset,
                    utc,
                })
            })
            .collect()
    }
}
