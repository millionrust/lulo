//! Cities the World Clock can add: name, country, IANA zone and position.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct City {
    pub name: &'static str,
    pub country: &'static str,
    pub zone: &'static str,
    pub latitude: f64,
    pub longitude: f64,
}

const fn city(
    name: &'static str,
    country: &'static str,
    zone: &'static str,
    latitude: f64,
    longitude: f64,
) -> City {
    City {
        name,
        country,
        zone,
        latitude,
        longitude,
    }
}

pub const CITIES: &[City] = &[
    city(
        "Abu Dhabi",
        "United Arab Emirates",
        "Asia/Dubai",
        24.45,
        54.38,
    ),
    city("Accra", "Ghana", "Africa/Accra", 5.60, -0.19),
    city("Addis Ababa", "Ethiopia", "Africa/Addis_Ababa", 9.03, 38.74),
    city(
        "Adelaide",
        "Australia",
        "Australia/Adelaide",
        -34.93,
        138.60,
    ),
    city("Ahmedabad", "India", "Asia/Kolkata", 23.02, 72.57),
    city("Algiers", "Algeria", "Africa/Algiers", 36.75, 3.06),
    city("Amsterdam", "Netherlands", "Europe/Amsterdam", 52.37, 4.90),
    city(
        "Anchorage",
        "United States",
        "America/Anchorage",
        61.22,
        -149.90,
    ),
    city("Athens", "Greece", "Europe/Athens", 37.98, 23.73),
    city(
        "Atlanta",
        "United States",
        "America/New_York",
        33.75,
        -84.39,
    ),
    city(
        "Auckland",
        "New Zealand",
        "Pacific/Auckland",
        -36.85,
        174.76,
    ),
    city("Baghdad", "Iraq", "Asia/Baghdad", 33.31, 44.36),
    city("Bangkok", "Thailand", "Asia/Bangkok", 13.76, 100.50),
    city("Barcelona", "Spain", "Europe/Madrid", 41.39, 2.17),
    city("Beijing", "China", "Asia/Shanghai", 39.90, 116.41),
    city("Beirut", "Lebanon", "Asia/Beirut", 33.89, 35.50),
    city("Bengaluru", "India", "Asia/Kolkata", 12.97, 77.59),
    city("Berlin", "Germany", "Europe/Berlin", 52.52, 13.40),
    city("Bogotá", "Colombia", "America/Bogota", 4.71, -74.07),
    city("Boston", "United States", "America/New_York", 42.36, -71.06),
    city(
        "Brisbane",
        "Australia",
        "Australia/Brisbane",
        -27.47,
        153.03,
    ),
    city("Brussels", "Belgium", "Europe/Brussels", 50.85, 4.35),
    city("Bucharest", "Romania", "Europe/Bucharest", 44.43, 26.10),
    city("Budapest", "Hungary", "Europe/Budapest", 47.50, 19.04),
    city(
        "Buenos Aires",
        "Argentina",
        "America/Argentina/Buenos_Aires",
        -34.60,
        -58.38,
    ),
    city("Cairo", "Egypt", "Africa/Cairo", 30.04, 31.24),
    city("Calgary", "Canada", "America/Edmonton", 51.05, -114.07),
    city(
        "Cape Town",
        "South Africa",
        "Africa/Johannesburg",
        -33.92,
        18.42,
    ),
    city("Caracas", "Venezuela", "America/Caracas", 10.48, -66.90),
    city("Casablanca", "Morocco", "Africa/Casablanca", 33.57, -7.59),
    city("Chennai", "India", "Asia/Kolkata", 13.08, 80.27),
    city("Chicago", "United States", "America/Chicago", 41.88, -87.63),
    city("Colombo", "Sri Lanka", "Asia/Colombo", 6.93, 79.86),
    city("Copenhagen", "Denmark", "Europe/Copenhagen", 55.68, 12.57),
    city(
        "Cupertino",
        "United States",
        "America/Los_Angeles",
        37.32,
        -122.03,
    ),
    city("Dallas", "United States", "America/Chicago", 32.78, -96.80),
    city(
        "Dar es Salaam",
        "Tanzania",
        "Africa/Dar_es_Salaam",
        -6.79,
        39.21,
    ),
    city("Delhi", "India", "Asia/Kolkata", 28.70, 77.10),
    city("Denver", "United States", "America/Denver", 39.74, -104.99),
    city("Dhaka", "Bangladesh", "Asia/Dhaka", 23.81, 90.41),
    city("Doha", "Qatar", "Asia/Qatar", 25.29, 51.53),
    city("Dubai", "United Arab Emirates", "Asia/Dubai", 25.20, 55.27),
    city("Dublin", "Ireland", "Europe/Dublin", 53.35, -6.26),
    city("Edinburgh", "United Kingdom", "Europe/London", 55.95, -3.19),
    city("Frankfurt", "Germany", "Europe/Berlin", 50.11, 8.68),
    city("Geneva", "Switzerland", "Europe/Zurich", 46.20, 6.14),
    city("Guangzhou", "China", "Asia/Shanghai", 23.13, 113.26),
    city("Hanoi", "Vietnam", "Asia/Bangkok", 21.03, 105.85),
    city("Havana", "Cuba", "America/Havana", 23.11, -82.37),
    city("Helsinki", "Finland", "Europe/Helsinki", 60.17, 24.94),
    city(
        "Ho Chi Minh City",
        "Vietnam",
        "Asia/Ho_Chi_Minh",
        10.82,
        106.63,
    ),
    city("Hong Kong", "China", "Asia/Hong_Kong", 22.32, 114.17),
    city(
        "Honolulu",
        "United States",
        "Pacific/Honolulu",
        21.31,
        -157.86,
    ),
    city("Houston", "United States", "America/Chicago", 29.76, -95.37),
    city("Hyderabad", "India", "Asia/Kolkata", 17.39, 78.49),
    city("Istanbul", "Türkiye", "Europe/Istanbul", 41.01, 28.98),
    city("Jakarta", "Indonesia", "Asia/Jakarta", -6.21, 106.85),
    city("Jerusalem", "Israel", "Asia/Jerusalem", 31.77, 35.21),
    city(
        "Johannesburg",
        "South Africa",
        "Africa/Johannesburg",
        -26.20,
        28.05,
    ),
    city("Kabul", "Afghanistan", "Asia/Kabul", 34.56, 69.21),
    city("Karachi", "Pakistan", "Asia/Karachi", 24.86, 67.01),
    city("Kathmandu", "Nepal", "Asia/Kathmandu", 27.72, 85.32),
    city("Kolkata", "India", "Asia/Kolkata", 22.57, 88.36),
    city(
        "Kuala Lumpur",
        "Malaysia",
        "Asia/Kuala_Lumpur",
        3.14,
        101.69,
    ),
    city("Kyiv", "Ukraine", "Europe/Kyiv", 50.45, 30.52),
    city("Lagos", "Nigeria", "Africa/Lagos", 6.52, 3.38),
    city("Lahore", "Pakistan", "Asia/Karachi", 31.55, 74.34),
    city(
        "Las Vegas",
        "United States",
        "America/Los_Angeles",
        36.17,
        -115.14,
    ),
    city("Lima", "Peru", "America/Lima", -12.05, -77.04),
    city("Lisbon", "Portugal", "Europe/Lisbon", 38.72, -9.14),
    city("London", "United Kingdom", "Europe/London", 51.51, -0.13),
    city(
        "Los Angeles",
        "United States",
        "America/Los_Angeles",
        34.05,
        -118.24,
    ),
    city("Madrid", "Spain", "Europe/Madrid", 40.42, -3.70),
    city("Manila", "Philippines", "Asia/Manila", 14.60, 120.98),
    city(
        "Melbourne",
        "Australia",
        "Australia/Melbourne",
        -37.81,
        144.96,
    ),
    city(
        "Mexico City",
        "Mexico",
        "America/Mexico_City",
        19.43,
        -99.13,
    ),
    city("Miami", "United States", "America/New_York", 25.76, -80.19),
    city("Milan", "Italy", "Europe/Rome", 45.46, 9.19),
    city(
        "Montevideo",
        "Uruguay",
        "America/Montevideo",
        -34.90,
        -56.16,
    ),
    city("Montreal", "Canada", "America/Toronto", 45.50, -73.57),
    city("Moscow", "Russia", "Europe/Moscow", 55.76, 37.62),
    city("Mumbai", "India", "Asia/Kolkata", 19.08, 72.88),
    city("Munich", "Germany", "Europe/Berlin", 48.14, 11.58),
    city("Nairobi", "Kenya", "Africa/Nairobi", -1.29, 36.82),
    city("New Delhi", "India", "Asia/Kolkata", 28.61, 77.21),
    city(
        "New York",
        "United States",
        "America/New_York",
        40.71,
        -74.01,
    ),
    city("Osaka", "Japan", "Asia/Tokyo", 34.69, 135.50),
    city("Oslo", "Norway", "Europe/Oslo", 59.91, 10.75),
    city("Ottawa", "Canada", "America/Toronto", 45.42, -75.70),
    city("Paris", "France", "Europe/Paris", 48.86, 2.35),
    city("Perth", "Australia", "Australia/Perth", -31.95, 115.86),
    city(
        "Phoenix",
        "United States",
        "America/Phoenix",
        33.45,
        -112.07,
    ),
    city("Prague", "Czechia", "Europe/Prague", 50.08, 14.44),
    city("Pune", "India", "Asia/Kolkata", 18.52, 73.86),
    city("Reykjavík", "Iceland", "Atlantic/Reykjavik", 64.15, -21.94),
    city(
        "Rio de Janeiro",
        "Brazil",
        "America/Sao_Paulo",
        -22.91,
        -43.17,
    ),
    city("Riyadh", "Saudi Arabia", "Asia/Riyadh", 24.71, 46.68),
    city("Rome", "Italy", "Europe/Rome", 41.90, 12.50),
    city(
        "San Francisco",
        "United States",
        "America/Los_Angeles",
        37.77,
        -122.42,
    ),
    city("Santiago", "Chile", "America/Santiago", -33.45, -70.67),
    city("São Paulo", "Brazil", "America/Sao_Paulo", -23.55, -46.63),
    city(
        "Seattle",
        "United States",
        "America/Los_Angeles",
        47.61,
        -122.33,
    ),
    city("Seoul", "South Korea", "Asia/Seoul", 37.57, 126.98),
    city("Shanghai", "China", "Asia/Shanghai", 31.23, 121.47),
    city("Shenzhen", "China", "Asia/Shanghai", 22.54, 114.06),
    city("Singapore", "Singapore", "Asia/Singapore", 1.35, 103.82),
    city("Stockholm", "Sweden", "Europe/Stockholm", 59.33, 18.07),
    city("Sydney", "Australia", "Australia/Sydney", -33.87, 151.21),
    city("Taipei", "Taiwan", "Asia/Taipei", 25.03, 121.57),
    city("Tehran", "Iran", "Asia/Tehran", 35.69, 51.39),
    city("Tel Aviv", "Israel", "Asia/Jerusalem", 32.09, 34.78),
    city("Tokyo", "Japan", "Asia/Tokyo", 35.68, 139.69),
    city("Toronto", "Canada", "America/Toronto", 43.65, -79.38),
    city("Vancouver", "Canada", "America/Vancouver", 49.28, -123.12),
    city("Vienna", "Austria", "Europe/Vienna", 48.21, 16.37),
    city("Warsaw", "Poland", "Europe/Warsaw", 52.23, 21.01),
    city(
        "Washington, D.C.",
        "United States",
        "America/New_York",
        38.91,
        -77.04,
    ),
    city(
        "Wellington",
        "New Zealand",
        "Pacific/Auckland",
        -41.29,
        174.78,
    ),
    city("Zürich", "Switzerland", "Europe/Zurich", 47.38, 8.54),
];

/// Case- and accent-insensitive match on the start of any word of the city
/// or its country, best matches (city-name prefix) first.
pub fn search(query: &str) -> Vec<&'static City> {
    let query = fold(query.trim());
    if query.is_empty() {
        return CITIES.iter().collect();
    }
    let word_prefix = |text: &str| {
        let text = fold(text);
        text.starts_with(&query)
            || text
                .split([' ', '-', ',', '.'])
                .any(|word| word.starts_with(&query))
    };
    let mut prefix: Vec<&City> = Vec::new();
    let mut other: Vec<&City> = Vec::new();
    for city in CITIES {
        if fold(city.name).starts_with(&query) {
            prefix.push(city);
        } else if word_prefix(city.name) || word_prefix(city.country) {
            other.push(city);
        }
    }
    prefix.extend(other);
    prefix
}

/// The city for a stored name, if it still exists.
pub fn find(name: &str) -> Option<&'static City> {
    CITIES.iter().find(|city| city.name == name)
}

/// The city to show first for the system zone: the zone's own city when it
/// is in the list. The Mac shows New Delhi, not Kolkata, for India.
pub fn for_zone(zone: &str) -> Option<&'static City> {
    if matches!(zone, "Asia/Kolkata" | "Asia/Calcutta") {
        return find("New Delhi");
    }
    let own = zone.rsplit('/').next().unwrap_or(zone).replace('_', " ");
    CITIES
        .iter()
        .find(|city| city.zone == zone && fold(city.name) == fold(&own))
        .or_else(|| CITIES.iter().find(|city| city.zone == zone))
}

fn fold(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'Á' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_sorted_unique_and_plausible() {
        let names = CITIES.iter().map(|city| city.name).collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_by_key(|name| fold(name));
        assert_eq!(names, sorted);
        sorted.dedup();
        assert_eq!(sorted.len(), CITIES.len());
        for city in CITIES {
            assert!(city.latitude.abs() <= 90.0 && city.longitude.abs() <= 180.0);
            assert!(city.zone.contains('/'), "{}", city.name);
        }
    }

    #[test]
    fn search_prefers_name_prefixes_and_ignores_accents() {
        let names = |query| {
            search(query)
                .iter()
                .map(|city| city.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("new")[..2], ["New Delhi", "New York"]);
        assert!(names("sao").contains(&"São Paulo"));
        assert!(names("zurich").contains(&"Zürich"));
        // Word prefixes and countries match after name prefixes.
        let delhi = names("delhi");
        assert_eq!(delhi[0], "Delhi");
        assert!(delhi.contains(&"New Delhi"));
        assert!(names("japan").contains(&"Tokyo"));
        assert!(names("xyzzy").is_empty());
        assert_eq!(search("").len(), CITIES.len());
    }

    #[test]
    fn system_zone_maps_to_a_city() {
        assert_eq!(for_zone("Asia/Kolkata").unwrap().name, "New Delhi");
        assert_eq!(for_zone("Europe/Paris").unwrap().name, "Paris");
        assert_eq!(for_zone("America/New_York").unwrap().name, "New York");
        assert_eq!(for_zone("America/Los_Angeles").unwrap().name, "Los Angeles");
        assert!(for_zone("Etc/UTC").is_none());
    }
}
