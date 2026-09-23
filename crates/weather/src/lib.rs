//! rmac Weather: forecasts from Open-Meteo for cities the user chooses.
//! Parsing, the request URLs, the hourly/ten-day presentation logic and the
//! saved places and cache are pure and unit-tested; the window is in the
//! binary.

pub mod fetch;
pub mod forecast;
pub mod geocode;
pub mod metrics;
pub mod store;
pub mod summary;
