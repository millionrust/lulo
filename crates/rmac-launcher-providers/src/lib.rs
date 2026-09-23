//! Local application, setting, file, calculator, conversion, currency,
//! world-clock and dictionary launcher providers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rmac_launcher::{
    Action, Cancellation, Category, Privacy, ProviderDescriptor, ProviderError, Request, ResultId,
    SearchResult,
};

#[cfg(test)]
mod answer_tests;
mod applications;
mod calculator;
pub mod conversion;
pub mod currency;
pub mod dictionary;
mod files;
mod helpers;
pub mod locale;
mod model;
mod search_in;
mod settings;
#[cfg(test)]
mod tests;
pub mod world_clock;

pub use applications::*;
pub use calculator::*;
pub use currency::{CurrencyProvider, EcbRates, RateSource, Rates};
pub use dictionary::DictionaryProvider;
pub use files::*;
use helpers::*;
use locale::Locale;
pub use model::*;
pub use search_in::*;
pub use settings::*;
pub use world_clock::{CityClock, CityTime, WorldClockProvider};

pub const APPLICATIONS_PROVIDER: &str = "applications";
pub const SETTINGS_PROVIDER: &str = "settings";
pub const FILES_PROVIDER: &str = "files";
pub const CALCULATOR_PROVIDER: &str = "calculator";
pub const CURRENCY_PROVIDER: &str = "currency";
pub const WORLD_CLOCK_PROVIDER: &str = "world-clock";
pub const DICTIONARY_PROVIDER: &str = "dictionary";
pub const SEARCH_IN_PROVIDER: &str = "search-in";

const PROVIDER_LIMIT: usize = 100;
