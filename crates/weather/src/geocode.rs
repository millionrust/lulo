//! City search through Open-Meteo's free geocoding API. rmac never asks
//! where the computer is; the user names the places they want.

use serde::{Deserialize, Serialize};

pub const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
pub const MAX_RESULTS: usize = 8;

/// A saved or found place.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Place {
    pub name: String,
    /// "Karnataka, India".
    #[serde(default)]
    pub region: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Place {
    /// Stable cache key: the position to two decimals (about a kilometre).
    pub fn key(&self) -> String {
        format!("{:.2}_{:.2}", self.latitude, self.longitude).replace('-', "m")
    }

    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.latitude.is_finite()
            && self.longitude.is_finite()
            && self.latitude.abs() <= 90.0
            && self.longitude.abs() <= 180.0
    }
}

/// Search URL, or `None` for a query too short to send.
pub fn search_url(query: &str) -> Option<String> {
    let query = query.trim();
    if query.chars().count() < 2 || query.len() > 100 {
        return None;
    }
    Some(format!(
        "{GEOCODING_ENDPOINT}?name={}&count={MAX_RESULTS}&language=en&format=json",
        percent_encode(query)
    ))
}

/// RFC 3986 percent-encoding of everything but unreserved characters.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[derive(Deserialize)]
struct RawResponse {
    #[serde(default)]
    results: Vec<RawPlace>,
}

#[derive(Deserialize)]
struct RawPlace {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    country: Option<String>,
}

/// Parse a search response; malformed entries are skipped.
pub fn parse_results(bytes: &[u8]) -> Option<Vec<Place>> {
    let raw: RawResponse = serde_json::from_slice(bytes).ok()?;
    Some(
        raw.results
            .into_iter()
            .map(|place| {
                let region = [place.admin1, place.country]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.trim().is_empty() && *part != place.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                Place {
                    name: place.name,
                    region,
                    latitude: place.latitude,
                    longitude: place.longitude,
                }
            })
            .filter(Place::is_valid)
            .take(MAX_RESULTS)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_urls_are_encoded_and_bounded() {
        assert_eq!(search_url(" "), None);
        assert_eq!(search_url("a"), None);
        assert_eq!(
            search_url("São Paulo").unwrap(),
            "https://geocoding-api.open-meteo.com/v1/search?name=S%C3%A3o%20Paulo&count=8&language=en&format=json"
        );
        assert!(search_url("a&b=c").unwrap().contains("name=a%26b%3Dc&"));
        assert_eq!(search_url(&"x".repeat(101)), None);
    }

    #[test]
    fn parses_results_with_regions() {
        let json = br#"{"results":[
          {"id":1277333,"name":"Bengaluru","latitude":12.97194,"longitude":77.59369,
           "country":"India","admin1":"Karnataka","timezone":"Asia/Kolkata"},
          {"id":2,"name":"Singapore","latitude":1.29,"longitude":103.85,
           "country":"Singapore","admin1":"Singapore"},
          {"id":3,"name":"Nowhere","latitude":123.0,"longitude":0.0}
        ],"generationtime_ms":0.5}"#;
        let places = parse_results(json).unwrap();
        assert_eq!(places.len(), 2);
        assert_eq!(places[0].name, "Bengaluru");
        assert_eq!(places[0].region, "Karnataka, India");
        assert_eq!(places[1].region, "");
        assert_eq!(places[0].key(), "12.97_77.59");
        assert_eq!(
            parse_results(br#"{"generationtime_ms":1}"#).unwrap(),
            vec![]
        );
        assert!(parse_results(b"nope").is_none());
    }

    #[test]
    fn cache_keys_are_filename_safe() {
        let place = Place {
            name: "Lima".into(),
            region: String::new(),
            latitude: -12.046,
            longitude: -77.043,
        };
        assert_eq!(place.key(), "m12.05_m77.04");
    }
}
