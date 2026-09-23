//! Currency conversion for the answer card: "10 usd in eur" = 8.77 EUR.
//!
//! macOS quotes Yahoo Finance. rmac uses the European Central Bank's daily
//! reference rates (a free, key-less XML file of about 30 currencies against
//! the euro), fetched through the system `curl` at most once every twelve
//! hours and only after a currency query, then cached on disk so answers
//! keep working offline with the date of the rates they use. The provider
//! is marked as using the network, so it stays off until the person allows
//! it in Settings › Spotlight.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::*;
use crate::conversion::{amount_and_words, connector_splits};
use crate::locale::{Locale, MONTHS};

pub const ECB_DAILY_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
/// Rates newer than this are used without asking again. The ECB publishes
/// once per working day around 16:00 CET.
const FRESH_FOR: Duration = Duration::from_secs(12 * 3_600);
/// After a failed or refused fetch, wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;
const CURRENCY_DECIMALS: usize = 2;

/// One day's reference rates: units of each currency per euro.
#[derive(Clone, Debug, PartialEq)]
pub struct Rates {
    /// `YYYY-MM-DD`, as published.
    pub date: String,
    pub per_euro: BTreeMap<String, f64>,
}

impl Rates {
    fn per_euro(&self, code: &str) -> Option<f64> {
        if code == "EUR" {
            Some(1.0)
        } else {
            self.per_euro.get(code).copied()
        }
    }

    pub fn knows(&self, code: &str) -> bool {
        self.per_euro(code).is_some()
    }

    /// `amount` of `from` in `to`.
    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        let value = amount / self.per_euro(from)? * self.per_euro(to)?;
        value.is_finite().then_some(value)
    }

    /// "23 Sep", the date the Mac-style source line shows.
    pub fn day_label(&self) -> Option<String> {
        let mut parts = self.date.split('-');
        let (_, month, day) = (parts.next()?, parts.next()?, parts.next()?);
        let month: usize = month.parse().ok()?;
        let day: u32 = day.parse().ok()?;
        Some(format!("{day} {}", MONTHS.get(month.checked_sub(1)?)?))
    }
}

/// Read the ECB's `eurofxref-daily.xml`:
/// `<Cube time='2026-09-22'><Cube currency='USD' rate='1.1734'/>…`.
pub fn parse_ecb(xml: &str) -> Option<Rates> {
    let date = attribute(xml, "time")?;
    if date.len() != 10
        || !date
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-')
    {
        return None;
    }
    let mut per_euro = BTreeMap::new();
    let mut rest = xml;
    while let Some(start) = rest.find("currency=") {
        rest = &rest[start..];
        let end = rest.find('>').unwrap_or(rest.len());
        let element = &rest[..end];
        if let (Some(code), Some(rate)) =
            (attribute(element, "currency"), attribute(element, "rate"))
        {
            if code.len() == 3 && code.chars().all(|character| character.is_ascii_uppercase()) {
                if let Ok(rate) = rate.parse::<f64>() {
                    if rate.is_finite() && rate > 0.0 {
                        per_euro.insert(code.to_owned(), rate);
                    }
                }
            }
        }
        rest = &rest[end..];
    }
    (!per_euro.is_empty()).then_some(Rates {
        date: date.to_owned(),
        per_euro,
    })
}

fn attribute<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut search = text;
    loop {
        let index = search.find(name)?;
        let after = &search[index + name.len()..];
        let before_ok = index == 0
            || !search[..index]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric());
        if before_ok {
            if let Some(value) = after.strip_prefix('=') {
                let quote = value.chars().next()?;
                if quote == '\'' || quote == '"' {
                    let value = &value[1..];
                    return value.find(quote).map(|end| &value[..end]);
                }
            }
        }
        search = after;
    }
}

/// Where the provider gets rates. The system source fetches and caches;
/// tests use fixed rates.
pub trait RateSource: Send + Sync + 'static {
    fn rates(&self, cancellation: &Cancellation) -> Option<Rates>;
}

/// Currency names and symbols people type, to ISO codes.
const NAMES: &[(&str, &str)] = &[
    ("$", "USD"),
    ("us$", "USD"),
    ("dollar", "USD"),
    ("dollars", "USD"),
    ("us dollar", "USD"),
    ("us dollars", "USD"),
    ("€", "EUR"),
    ("euro", "EUR"),
    ("euros", "EUR"),
    ("£", "GBP"),
    ("pound sterling", "GBP"),
    ("pounds sterling", "GBP"),
    ("quid", "GBP"),
    ("¥", "JPY"),
    ("yen", "JPY"),
    ("₹", "INR"),
    ("rupee", "INR"),
    ("rupees", "INR"),
    ("rs", "INR"),
    ("yuan", "CNY"),
    ("renminbi", "CNY"),
    ("franc", "CHF"),
    ("francs", "CHF"),
    ("swiss franc", "CHF"),
    ("swiss francs", "CHF"),
    ("won", "KRW"),
    ("real", "BRL"),
    ("reais", "BRL"),
    ("rand", "ZAR"),
    ("zloty", "PLN"),
    ("lira", "TRY"),
    ("krona", "SEK"),
    ("kronor", "SEK"),
    ("canadian dollar", "CAD"),
    ("canadian dollars", "CAD"),
    ("australian dollar", "AUD"),
    ("australian dollars", "AUD"),
    ("singapore dollar", "SGD"),
    ("singapore dollars", "SGD"),
    ("hong kong dollar", "HKD"),
    ("hong kong dollars", "HKD"),
];

/// ISO code for what was typed: "usd", "USD", "$", "dollars".
pub fn currency_code(text: &str, rates: &Rates) -> Option<String> {
    let text = text.trim().trim_end_matches('.');
    let lower = text.to_lowercase();
    if let Some((_, code)) = NAMES.iter().find(|(name, _)| *name == lower) {
        return Some((*code).to_owned());
    }
    let upper = text.to_ascii_uppercase();
    (upper.len() == 3
        && upper
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        && rates.knows(&upper))
    .then_some(upper)
}

/// A currency query: amount, source code, target code. Accepts "10 usd in
/// eur", "10usd to inr", "$10 in eur", "€5 → gbp", "10 dollars in rupees".
pub fn parse_currency_query(query: &str, rates: &Rates) -> Option<(f64, String, String)> {
    let query = query.trim();
    // A leading symbol: "$10 in eur".
    let (symbol, rest) = ["$", "€", "£", "¥", "₹"]
        .iter()
        .find_map(|symbol| query.strip_prefix(symbol).map(|rest| (Some(*symbol), rest)))
        .unwrap_or((None, query));
    let (amount, words) = amount_and_words(rest)?;
    if let Some(symbol) = symbol {
        // "$10 in eur": the words are "in eur".
        let (connector, target) = words.split_first()?;
        if !crate::conversion::CONNECTORS.contains(&connector.to_lowercase().as_str()) {
            return None;
        }
        let from = currency_code(symbol, rates)?;
        let to = currency_code(&target.join(" "), rates)?;
        return (from != to).then_some((amount, from, to));
    }
    connector_splits(&words).into_iter().find_map(|(from, to)| {
        let (from, to) = (currency_code(&from, rates)?, currency_code(&to, rates)?);
        (from != to).then_some((amount, from, to))
    })
}

/// "8.77 EUR" for "10 usd in eur".
pub fn convert_currency(query: &str, rates: &Rates, locale: &Locale) -> Option<String> {
    let (amount, from, to) = parse_currency_query(query, rates)?;
    let value = rates.convert(amount, &from, &to)?;
    Some(format!("{} {to}", locale.format(value, CURRENCY_DECIMALS)?))
}

/// Cheap check before any rates are loaded: does the query look like
/// "<amount> <currency> <connector> <currency>"?
fn looks_like_currency(query: &str) -> bool {
    let query = query.trim();
    let symbol = ["$", "€", "£", "¥", "₹"]
        .iter()
        .any(|symbol| query.starts_with(symbol));
    let rest = ["$", "€", "£", "¥", "₹"]
        .iter()
        .find_map(|symbol| query.strip_prefix(symbol))
        .unwrap_or(query);
    let Some((_, words)) = amount_and_words(rest) else {
        return false;
    };
    let has_connector = words
        .iter()
        .any(|word| crate::conversion::CONNECTORS.contains(&word.to_lowercase().as_str()));
    has_connector
        && (symbol || !connector_splits(&words).is_empty())
        && crate::conversion::lookup(words.first().copied().unwrap_or_default()).is_none()
}

pub struct CurrencyProvider<S = EcbRates> {
    source: S,
}

impl<S> CurrencyProvider<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }

    #[cfg(test)]
    pub(crate) fn source(&self) -> &S {
        &self.source
    }
}

impl<S: RateSource> Provider for CurrencyProvider<S> {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            CURRENCY_PROVIDER,
            Category::Calculator,
            Privacy {
                private_content: false,
                network: true,
            },
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        if !looks_like_currency(query) {
            return Ok(Vec::new());
        }
        let Some(rates) = self.source.rates(cancellation) else {
            return Ok(Vec::new());
        };
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let locale = Locale::from_environment();
        Ok(convert_currency(query, &rates, &locale)
            .map(|text| {
                let source = match rates.day_label() {
                    Some(day) => format!("European Central Bank · {day}"),
                    None => "European Central Bank".to_owned(),
                };
                calculator::answer_result(CURRENCY_PROVIDER, query, text, Some(source))
            })
            .into_iter()
            .collect())
    }
}

/// The ECB daily file, cached at `$XDG_CACHE_HOME/rmac/ecb-eurofxref-daily.xml`.
pub struct EcbRates {
    cache: Option<PathBuf>,
    memory: Mutex<Option<(Rates, SystemTime)>>,
    fetching: AtomicBool,
    /// When the last fetch was tried, so an offline session does not try
    /// again on every keystroke.
    last_attempt: Mutex<Option<SystemTime>>,
}

impl EcbRates {
    pub fn new(cache: Option<PathBuf>) -> Self {
        Self {
            cache,
            memory: Mutex::new(None),
            fetching: AtomicBool::new(false),
            last_attempt: Mutex::new(None),
        }
    }

    fn may_attempt(&self) -> bool {
        let Ok(mut last) = self.last_attempt.lock() else {
            return false;
        };
        let now = SystemTime::now();
        if last.is_some_and(|last| {
            now.duration_since(last)
                .is_ok_and(|elapsed| elapsed < RETRY_AFTER)
        }) {
            return false;
        }
        *last = Some(now);
        true
    }

    /// The per-user cache location, if the session names a home.
    pub fn default_cache() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|home| home.join(".cache"))
            })?;
        Some(base.join("rmac").join("ecb-eurofxref-daily.xml"))
    }

    fn cached(&self) -> Option<(Rates, SystemTime)> {
        if let Some(memory) = self.memory.lock().ok()?.clone() {
            return Some(memory);
        }
        let path = self.cache.as_ref()?;
        let modified = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        let rates = parse_ecb(&text)?;
        if let Ok(mut memory) = self.memory.lock() {
            *memory = Some((rates.clone(), modified));
        }
        Some((rates, modified))
    }

    fn fetch(&self) -> Option<Rates> {
        let mut child = Command::new("curl")
            .args([
                "--silent",
                "--fail",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "8",
                "--max-filesize",
                "262144",
                "--",
                ECB_DAILY_URL,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut body = String::new();
        let read = child
            .stdout
            .take()
            .map(|stdout| stdout.take(MAX_RESPONSE_BYTES).read_to_string(&mut body));
        let status = child.wait().ok()?;
        if !status.success() || !matches!(read, Some(Ok(_))) {
            return None;
        }
        let rates = parse_ecb(&body)?;
        if let Some(path) = &self.cache {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let temporary = path.with_extension("xml.part");
            if std::fs::write(&temporary, body.as_bytes()).is_ok() {
                let _ = std::fs::rename(&temporary, path);
            }
        }
        if let Ok(mut memory) = self.memory.lock() {
            *memory = Some((rates.clone(), SystemTime::now()));
        }
        Some(rates)
    }
}

impl RateSource for EcbRates {
    fn rates(&self, cancellation: &Cancellation) -> Option<Rates> {
        let cached = self.cached();
        let fresh = cached
            .as_ref()
            .is_some_and(|(_, fetched)| fetched.elapsed().is_ok_and(|elapsed| elapsed < FRESH_FOR));
        if fresh || cancellation.is_cancelled() || !self.may_attempt() {
            return cached.map(|(rates, _)| rates);
        }
        // One fetch at a time; other keystrokes use what is cached.
        if self
            .fetching
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return cached.map(|(rates, _)| rates);
        }
        let fetched = self.fetch();
        self.fetching.store(false, Ordering::Release);
        fetched.or_else(|| cached.map(|(rates, _)| rates))
    }
}
