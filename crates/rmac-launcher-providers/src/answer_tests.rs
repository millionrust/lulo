//! Answers Spotlight computes from the query: numbers, units, currency,
//! world clock, definitions, file-row details and the "Search in" row.
//! Expected strings come from the owner's Mac (design-lab/spotlight.html).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::currency::{convert_currency, parse_currency_query, parse_ecb};
use crate::dictionary::{
    decode_b64, first_definition, installed_dictionaries, word_to_define, Dictionary,
};
use crate::locale::{DateOrder, Grouping};
use crate::world_clock::{
    city_result, gmt_offset, offset_difference, place_in_time_query, relative_day,
};

fn india() -> Locale {
    Locale::from_tag("en_IN.UTF-8")
}

fn calc(query: &str) -> Option<String> {
    answer(query, &india())
}

// ---------------------------------------------------------------- locale

#[test]
fn locale_tags_choose_grouping_separators_and_date_order() {
    let india = india();
    assert_eq!(india.grouping, Grouping::Indian);
    assert_eq!((india.decimal, india.group), ('.', ','));
    assert_eq!(india.date_order, DateOrder::DayMonth);
    let us = Locale::from_tag("en_US.UTF-8");
    assert_eq!(us.grouping, Grouping::Thousands);
    assert_eq!(us.date_order, DateOrder::MonthDay);
    let german = Locale::from_tag("de_DE@euro");
    assert_eq!((german.decimal, german.group), (',', '.'));
    let french = Locale::from_tag("fr_FR");
    assert_eq!((french.decimal, french.group), (',', '\u{202f}'));
    let brazil = Locale::from_tag("pt_BR");
    assert_eq!((brazil.decimal, brazil.group), (',', '.'));
}

#[test]
fn numbers_group_like_the_mac() {
    let india = india();
    let british = Locale::from_tag("en_GB");
    assert_eq!(india.format(1_000_000.0, 10).unwrap(), "10,00,000");
    assert_eq!(india.format(12_345_678.0, 10).unwrap(), "1,23,45,678");
    assert_eq!(british.format(12_345_678.0, 10).unwrap(), "12,345,678");
    assert_eq!(india.format(1_024.0, 10).unwrap(), "1,024");
    assert_eq!(india.format(123.0, 10).unwrap(), "123");
    assert_eq!(india.format(4_046.856_422_4, 2).unwrap(), "4,046.86");
    assert_eq!(india.format(-1_234.5, 2).unwrap(), "-1,234.5");
    assert_eq!(india.format(-0.0, 2).unwrap(), "0");
    assert_eq!(india.format(-0.001, 2).unwrap(), "0");
    assert_eq!(
        Locale::from_tag("de_DE").format(1_234.5, 2).unwrap(),
        "1.234,5"
    );
    assert_eq!(
        Locale::from_tag("fr_FR").format(1_234.5, 2).unwrap(),
        "1\u{202f}234,5"
    );
    assert_eq!(india.format(f64::NAN, 2), None);
    assert_eq!(india.format(f64::INFINITY, 2), None);
}

#[test]
fn huge_and_tiny_numbers_use_scientific_notation() {
    let india = india();
    assert_eq!(india.format(1e20, 10).unwrap(), "1e20");
    assert_eq!(
        india.format(2_f64.powi(100), 10).unwrap(),
        "1.2676506002e30"
    );
    assert_eq!(india.format(1e-12, 10).unwrap(), "1e-12");
    assert_eq!(
        Locale::from_tag("de_DE").format(1.5e20, 10).unwrap(),
        "1,5e20"
    );
}

#[test]
fn short_dates_follow_the_region() {
    assert_eq!(india().short_date(2026, 9, 17), "17/09/26");
    assert_eq!(Locale::from_tag("en_US").short_date(2026, 9, 17), "9/17/26");
    assert_eq!(locale::time_12h(0, 5), "12:05 AM");
    assert_eq!(locale::time_12h(12, 0), "12:00 PM");
    assert_eq!(locale::time_12h(15, 33), "3:33 PM");
    assert_eq!(locale::civil_from_days(20_719), (2026, 9, 23));
    assert_eq!(locale::civil_from_days(0), (1970, 1, 1));
}

// ------------------------------------------------------------ calculator

#[test]
fn calculations_match_the_mac() {
    for (query, expected) in [
        ("12*7", "84"),
        ("1000*1000", "10,00,000"),
        ("2^10", "1,024"),
        ("sqrt(2)", "1.4142135624"),
        ("1/3", "0.3333333333"),
        ("2**3", "8"),
        ("-2^2", "-4"),
        ("2^3^2", "512"),
        ("2^-1", "0.5"),
        ("(2+3)×4", "20"),
        ("10 ÷ 4", "2.5"),
        ("pi*2", "6.2831853072"),
        ("abs(-3) + log(100)", "5"),
        ("ln(e)", "1"),
        ("  7 − 10 ", "-3"),
    ] {
        assert_eq!(calc(query).as_deref(), Some(expected), "{query}");
    }
}

#[test]
fn non_calculations_and_invalid_expressions_have_no_answer() {
    for query in [
        "",
        "5",
        "hello",
        "2 +",
        "1/0",
        "sqrt(-1)",
        "ln(0)",
        "(2",
        "2)",
        "2(3)",
        "20% of 150",
        "sqrt 2",
        ".",
        "1e5",
        "terminal",
    ] {
        assert_eq!(calc(query), None, "{query}");
    }
    assert_eq!(calc(&"1+".repeat(200)), None);
}

// ----------------------------------------------------------- conversions

#[test]
fn unit_conversions_match_the_mac() {
    for (query, expected) in [
        ("5 km in miles", "3.11 miles"),
        ("100 f in c", "37.78 °C"),
        ("5 kg in lb", "11.023 lb"),
        ("1 gb in mb", "1,000 MB"),
        ("60 mph in km/h", "96.56 km/h"),
        ("3 hours in minutes", "180 minutes"),
        ("1 acre in m2", "4,046.86 m²"),
    ] {
        assert_eq!(calc(query).as_deref(), Some(expected), "{query}");
    }
}

#[test]
fn every_dimension_converts() {
    for (query, expected) in [
        // Length
        ("12 in in cm", "30.48 cm"),
        ("5km to mi", "3.11 mi"),
        ("1.609344 km in miles", "1 mile"),
        ("6 ft in m", "1.83 m"),
        ("1 nautical mile in km", "1.85 km"),
        // Mass
        ("1 lb in g", "453.592 g"),
        ("16 oz to lb", "1 lb"),
        ("2 stone in kg", "12.701 kg"),
        // Temperature
        ("0 c in f", "32 °F"),
        ("-40 c to f", "-40 °F"),
        ("300 k in c", "26.85 °C"),
        ("98.6 fahrenheit in celsius", "37 °C"),
        // Volume
        ("2 l in ml", "2,000 mL"),
        ("1 gal in l", "3.79 L"),
        ("1 cup in ml", "236.59 mL"),
        ("1 imperial gallon in litres", "4.55 litres"),
        // Speed
        ("10 m/s in km/h", "36 km/h"),
        ("20 knots in mph", "23.02 mph"),
        // Area
        ("1 ha in acres", "2.47 acres"),
        ("100 sq ft in m2", "9.29 m²"),
        // Time
        ("1 day in hours", "24 hours"),
        ("90 min in hours", "1.5 hours"),
        ("1 week in days", "7 days"),
        // Data
        ("1 gib in mb", "1,073.74 MB"),
        ("8 bits in bytes", "1 byte"),
        ("1,500 kb to mb", "1.5 MB"),
    ] {
        assert_eq!(calc(query).as_deref(), Some(expected), "{query}");
    }
}

#[test]
fn mismatched_or_unknown_units_have_no_answer() {
    for query in [
        "5 km in kg",
        "5 km",
        "5 foo in bar",
        "5 km in km",
        "km in miles",
        "5 km in",
        "in 5 km",
    ] {
        assert_eq!(calc(query), None, "{query}");
    }
}

#[test]
fn unit_spellings_resolve_to_one_unit() {
    for (text, symbol, spelled) in [
        ("km", "km", false),
        ("kilometres", "km", true),
        ("kilometers", "km", true),
        ("miles", "mi", true),
        ("mi", "mi", false),
        ("°C", "°C", false),
        ("c", "°C", false),
        ("celsius", "°C", false),
        ("m²", "m²", false),
        ("sq ft", "ft²", false),
        ("MB", "MB", false),
        ("mb", "MB", false),
        ("mL", "mL", false),
        ("minutes", "min", true),
    ] {
        let written = conversion::lookup(text).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(written.unit.symbol, symbol, "{text}");
        assert_eq!(written.spelled, spelled, "{text}");
    }
    assert!(conversion::lookup("parsec").is_none());
    assert!(conversion::lookup("").is_none());
}

#[test]
fn calculator_provider_echoes_the_query_for_the_card() {
    let results = CalculatorProvider
        .search(" 12*7 ", &Cancellation::default())
        .expect("calculator succeeds");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "84");
    assert_eq!(results[0].subtitle.as_deref(), Some("12*7"));
    assert_eq!(results[0].category, Category::Calculator);
    assert_eq!(results[0].primary, Action::CopyText { text: "84".into() });
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(CalculatorProvider.search("12*7", &cancelled).is_err());
}

// --------------------------------------------------------------- currency

const ECB_SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope xmlns:gesmes="http://www.gesmes.org/xml/2002-08-01" xmlns="http://www.ecb.int/vocabulary/2002-08-01/eurofxref">
	<gesmes:subject>Reference rates</gesmes:subject>
	<gesmes:Sender>
		<gesmes:name>European Central Bank</gesmes:name>
	</gesmes:Sender>
	<Cube>
		<Cube time='2026-09-22'>
			<Cube currency='USD' rate='1.14'/>
			<Cube currency='JPY' rate='160.0'/>
			<Cube currency='GBP' rate='0.85'/>
			<Cube currency="INR" rate="95.0"/>
			<Cube currency='BAD' rate='-1'/>
			<Cube currency='xx' rate='2'/>
		</Cube>
	</Cube>
</gesmes:Envelope>"#;

fn rates() -> Rates {
    parse_ecb(ECB_SAMPLE).expect("sample parses")
}

#[test]
fn ecb_reference_rates_parse() {
    let rates = rates();
    assert_eq!(rates.date, "2026-09-22");
    assert_eq!(
        rates
            .per_euro
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["GBP", "INR", "JPY", "USD"]
    );
    assert_eq!(rates.day_label().as_deref(), Some("22 Sep"));
    assert!(rates.knows("EUR"));
    assert!(!rates.knows("BAD"));
    assert_eq!(parse_ecb("<Cube time='2026-09-22'></Cube>"), None);
    assert_eq!(parse_ecb("not xml"), None);
}

#[test]
fn currency_conversions_read_codes_symbols_and_names() {
    let rates = rates();
    let india = india();
    for (query, expected) in [
        ("10 usd in eur", "8.77 EUR"),
        ("10 USD to EUR", "8.77 EUR"),
        ("$10 in eur", "8.77 EUR"),
        ("10$ in eur", "8.77 EUR"),
        ("10 dollars to rupees", "833.33 INR"),
        ("€5 → gbp", "4.25 GBP"),
        ("1000 eur in inr", "95,000 INR"),
        ("100 jpy in usd", "0.71 USD"),
    ] {
        assert_eq!(
            convert_currency(query, &rates, &india).as_deref(),
            Some(expected),
            "{query}"
        );
    }
    for query in [
        "10 usd in usd",
        "10 usd in xyz",
        "10 abc in eur",
        "usd in eur",
        "10 usd",
    ] {
        assert_eq!(convert_currency(query, &rates, &india), None, "{query}");
    }
    assert_eq!(
        parse_currency_query("2.5 gbp as usd", &rates),
        Some((2.5, "GBP".into(), "USD".into()))
    );
}

struct FixedRates {
    rates: Option<Rates>,
    calls: AtomicUsize,
}

impl RateSource for FixedRates {
    fn rates(&self, _: &Cancellation) -> Option<Rates> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.rates.clone()
    }
}

#[test]
fn currency_provider_needs_network_permission_and_only_asks_for_rates_when_needed() {
    let provider = CurrencyProvider::new(FixedRates {
        rates: Some(rates()),
        calls: AtomicUsize::new(0),
    });
    let descriptor = provider.descriptor();
    assert!(descriptor.privacy.network);
    assert!(!descriptor.privacy.private_content);
    assert_eq!(descriptor.category, Category::Calculator);

    for query in ["5 kg in lb", "12*7", "hello", "terminal"] {
        assert!(provider
            .search(query, &Cancellation::default())
            .unwrap()
            .is_empty());
    }
    assert_eq!(provider.source_calls(), 0);

    let results = provider
        .search("10 usd in eur", &Cancellation::default())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.ends_with(" EUR"));
    assert_eq!(results[0].subtitle.as_deref(), Some("10 usd in eur"));
    assert_eq!(
        results[0].detail.as_deref(),
        Some("European Central Bank · 22 Sep")
    );
    assert_eq!(provider.source_calls(), 1);

    let offline = CurrencyProvider::new(FixedRates {
        rates: None,
        calls: AtomicUsize::new(0),
    });
    assert!(offline
        .search("10 usd in eur", &Cancellation::default())
        .unwrap()
        .is_empty());
}

impl CurrencyProvider<FixedRates> {
    fn source_calls(&self) -> usize {
        self.source().calls.load(Ordering::SeqCst)
    }
}

#[test]
fn cached_rates_answer_offline() {
    let directory = temporary_directory("rates");
    std::fs::create_dir_all(&directory).unwrap();
    let cache = directory.join("ecb-eurofxref-daily.xml");
    std::fs::write(&cache, ECB_SAMPLE).unwrap();
    let source = EcbRates::new(Some(cache));
    // The cache file was just written, so it is fresh: no fetch is tried.
    let rates = source
        .rates(&Cancellation::default())
        .expect("cached rates are used");
    assert_eq!(rates.date, "2026-09-22");
    std::fs::remove_dir_all(directory).unwrap();

    let nothing = EcbRates::new(None);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(nothing.rates(&cancelled), None);
}

// ------------------------------------------------------------ world clock

#[test]
fn time_queries_name_a_place() {
    for (query, place) in [
        ("time in tokyo", "tokyo"),
        ("Time in Tokyo", "tokyo"),
        ("What time is it in New York?", "new york"),
        ("current time in  paris", "paris"),
        ("tokyo time", "tokyo"),
        ("time london", "london"),
    ] {
        assert_eq!(
            place_in_time_query(query).as_deref(),
            Some(place),
            "{query}"
        );
    }
    for query in ["time", "time in t", "timer", "tokyo", "12*7"] {
        assert_eq!(place_in_time_query(query), None, "{query}");
    }
}

fn tokyo_at(utc: i64) -> CityTime {
    CityTime {
        city: "Tokyo".into(),
        country: "Japan".into(),
        zone: "Asia/Tokyo".into(),
        offset: 9 * 3_600,
        local_offset: 19_800,
        utc,
    }
}

#[test]
fn the_time_card_reads_like_the_mac() {
    // 2026-09-23 15:46 UTC: 12:46 AM on the 24th in Tokyo, 9:16 PM in
    // New Delhi (the Mac capture's moment).
    let utc = 20_719 * 86_400 + 15 * 3_600 + 46 * 60;
    let result = city_result(&tokyo_at(utc));
    assert_eq!(result.title, "Tokyo, Japan");
    assert_eq!(
        result.subtitle.as_deref(),
        Some("GMT+9 · Tomorrow, +3:30 HRS")
    );
    assert_eq!(result.detail.as_deref(), Some("12:46 AM"));
    assert_eq!(result.category, Category::Clock);
    assert_eq!(
        result.primary,
        Action::CopyText {
            text: "12:46 AM".into()
        }
    );

    assert_eq!(gmt_offset(0), "GMT");
    assert_eq!(gmt_offset(19_800), "GMT+5:30");
    assert_eq!(gmt_offset(-3 * 3_600), "GMT-3");
    assert_eq!(offset_difference(3_600, 0), "+1 HR");
    assert_eq!(offset_difference(-4 * 3_600, 0), "-4 HRS");
    assert_eq!(offset_difference(0, 0), "+0 HRS");
    assert_eq!(relative_day(9, 10), "Yesterday");
    assert_eq!(relative_day(10, 10), "Today");
}

struct FakeClock;

impl CityClock for FakeClock {
    fn cities(&self, name: &str) -> Vec<CityTime> {
        if "tokyo".starts_with(name) {
            vec![tokyo_at(0)]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn world_clock_provider_answers_only_time_queries() {
    let provider = WorldClockProvider::new(FakeClock);
    assert_eq!(provider.descriptor().category, Category::Clock);
    let results = provider
        .search("time in tokyo", &Cancellation::default())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Tokyo, Japan");
    for query in ["tokyo", "time in atlantis", ""] {
        assert!(provider
            .search(query, &Cancellation::default())
            .unwrap()
            .is_empty());
    }
}

// ------------------------------------------------------------- dictionary

#[test]
fn dictd_numbers_decode() {
    assert_eq!(decode_b64(b"A"), Some(0));
    assert_eq!(decode_b64(b"B"), Some(1));
    assert_eq!(decode_b64(b"BA"), Some(64));
    assert_eq!(decode_b64(b"/"), Some(63));
    assert_eq!(decode_b64(b"Bk\r"), Some(64 + 36));
    assert_eq!(decode_b64(b""), None);
    assert_eq!(decode_b64(b"*"), None);
}

#[test]
fn definition_queries_name_one_word() {
    for (query, word) in [
        ("define serendipity", "serendipity"),
        ("Definition of Serendipity?", "serendipity"),
        ("what does ephemeral mean", "ephemeral"),
        ("serendipity meaning", "serendipity"),
        ("define ice cream", "ice cream"),
    ] {
        assert_eq!(word_to_define(query).as_deref(), Some(word), "{query}");
    }
    for query in ["define", "define 123", "terminal", "define a/b"] {
        assert_eq!(word_to_define(query), None, "{query}");
    }
}

const SERENDIPITY: &str =
    "serendipity\n     n : good luck in making unexpected and fortunate discoveries\n";
const DOG: &str = "dog\n     n 1: a member of the genus Canis (probably descended from the\n          common wolf) that has been domesticated by man since\n          prehistoric times; \"the dog barked all night\" [syn: {domestic\n          dog}, {Canis familiaris}]\n     2: a dull unattractive unpleasant girl or woman; \"she got a\n        reputation as a frump\"\n";

#[test]
fn first_definitions_come_out_on_one_line() {
    assert_eq!(
        first_definition(SERENDIPITY).as_deref(),
        Some("good luck in making unexpected and fortunate discoveries")
    );
    assert_eq!(
        first_definition(DOG).as_deref(),
        Some(
            "a member of the genus Canis (probably descended from the common wolf) that has \
             been domesticated by man since prehistoric times"
        )
    );
    assert_eq!(
        first_definition("Serendip \\Ser\"en*dip\\, n.\n   An old name of Ceylon.\n").as_deref(),
        Some("An old name of Ceylon.")
    );
    assert_eq!(first_definition("word\n"), None);
}

/// dictd base 64, most significant digit first.
fn encode_b64(mut value: u64) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut digits = Vec::new();
    loop {
        digits.push(ALPHABET[(value % 64) as usize]);
        value /= 64;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).unwrap()
}

/// A plain dictd database (headword, entry text) and its index.
fn write_database(directory: &Path, stem: &str, entries: &[(&str, &str)]) -> (Vec<u8>, String) {
    let mut data = Vec::new();
    let mut index = String::new();
    for (headword, entry) in entries {
        index.push_str(&format!(
            "{headword}\t{}\t{}\n",
            encode_b64(data.len() as u64),
            encode_b64(entry.len() as u64)
        ));
        data.extend_from_slice(entry.as_bytes());
    }
    std::fs::write(directory.join(format!("{stem}.index")), &index).unwrap();
    (data, index)
}

/// dictzip: gzip with an RA field and independently inflatable chunks.
fn dictzip(data: &[u8], chunk_length: usize) -> Vec<u8> {
    let mut chunks = Vec::new();
    for chunk in data.chunks(chunk_length) {
        let mut compressor = flate2::Compress::new(flate2::Compression::default(), false);
        let mut output = Vec::with_capacity(chunk.len() * 2 + 64);
        compressor
            .compress_vec(chunk, &mut output, flate2::FlushCompress::Sync)
            .unwrap();
        chunks.push(output);
    }
    let mut extra = Vec::new();
    extra.extend_from_slice(b"RA");
    extra.extend_from_slice(&((6 + 2 * chunks.len()) as u16).to_le_bytes());
    extra.extend_from_slice(&1_u16.to_le_bytes());
    extra.extend_from_slice(&(chunk_length as u16).to_le_bytes());
    extra.extend_from_slice(&(chunks.len() as u16).to_le_bytes());
    for chunk in &chunks {
        extra.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
    }
    // FEXTRA and FNAME set.
    let mut file = vec![0x1f, 0x8b, 8, 0x04 | 0x08, 0, 0, 0, 0, 2, 3];
    file.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    file.extend_from_slice(&extra);
    file.extend_from_slice(b"wn.dict\0");
    for chunk in &chunks {
        file.extend_from_slice(chunk);
    }
    file.extend_from_slice(&[0; 8]);
    file
}

#[test]
fn installed_dictionaries_are_found_and_read_plain_or_dictzipped() {
    let directory = temporary_directory("dictd");
    std::fs::create_dir_all(&directory).unwrap();
    let (wordnet, _) = write_database(
        &directory,
        "wn",
        &[("dog", DOG), ("serendipity", SERENDIPITY)],
    );
    // 16-byte chunks: every entry spans several.
    std::fs::write(directory.join("wn.dict.dz"), dictzip(&wordnet, 16)).unwrap();
    let (gcide, _) = write_database(
        &directory,
        "gcide",
        &[(
            "Serendip",
            "Serendip \\Ser\"en*dip\\, n.\n   An old name of Ceylon.\n",
        )],
    );
    std::fs::write(directory.join("gcide.dict"), gcide).unwrap();
    write_database(&directory, "freedict-eng-deu", &[("dog", "dog\n   Hund\n")]);
    std::fs::write(directory.join("freedict-eng-deu.dict"), b"dog\n   Hund\n").unwrap();
    // An index without data is ignored.
    std::fs::write(directory.join("orphan.index"), "x\tA\tB\n").unwrap();

    let found = installed_dictionaries(&directory);
    assert_eq!(
        found
            .iter()
            .map(|found| found.name.as_str())
            .collect::<Vec<_>>(),
        ["WordNet", "GCIDE"]
    );
    let provider = DictionaryProvider::new(found.clone());
    assert!(provider.is_available());

    let results = provider
        .search("define serendipity", &Cancellation::default())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "serendipity");
    assert_eq!(
        results[0].subtitle.as_deref(),
        Some("good luck in making unexpected and fortunate discoveries")
    );
    assert_eq!(results[0].detail.as_deref(), Some("WordNet"));
    assert_eq!(results[0].category, Category::Dictionary);

    // Case-insensitive headwords; a later dictionary answers what WordNet
    // lacks.
    let results = provider
        .search("define serendip", &Cancellation::default())
        .unwrap();
    assert_eq!(results[0].detail.as_deref(), Some("GCIDE"));
    assert_eq!(
        results[0].subtitle.as_deref(),
        Some("An old name of Ceylon.")
    );

    assert!(provider
        .search("define unobtainium", &Cancellation::default())
        .unwrap()
        .is_empty());
    assert!(provider
        .search("serendipity", &Cancellation::default())
        .unwrap()
        .is_empty());
    let plain = Dictionary {
        name: "WordNet".into(),
        index: directory.join("wn.index"),
        data: directory.join("wn.dict.dz"),
    };
    assert!(dictionary::lookup(&plain, "DOG", &Cancellation::default())
        .unwrap()
        .starts_with("a member of the genus Canis"));
    std::fs::remove_dir_all(directory).unwrap();

    assert!(!DictionaryProvider::new(Vec::new()).is_available());
}

// ------------------------------------------------------ files and search-in

#[test]
fn file_sizes_and_dates_read_like_finder() {
    assert_eq!(file_size(1), "1 byte");
    assert_eq!(file_size(926), "926 bytes");
    assert_eq!(file_size(24_123), "24 KB");
    assert_eq!(file_size(1_000), "1 KB");
    assert_eq!(file_size(1_300_000), "1.3 MB");
    assert_eq!(file_size(2_000_000), "2 MB");
    assert_eq!(file_size(2_150_000_000), "2.15 GB");

    let today = (2026, 9, 23);
    let yesterday = Some((2026, 9, 22));
    let british = Locale::from_tag("en_GB");
    assert_eq!(
        file_date(today, (21, 8), today, yesterday, &british),
        "Today, 9:08 PM"
    );
    assert_eq!(
        file_date((2026, 9, 22), (20, 20), today, yesterday, &british),
        "Yesterday, 8:20 PM"
    );
    assert_eq!(
        file_date((2026, 9, 17), (15, 33), today, yesterday, &british),
        "17/09/26, 3:33 PM"
    );
    assert_eq!(
        file_date(
            (2026, 9, 17),
            (15, 33),
            today,
            yesterday,
            &Locale::from_tag("en_US")
        ),
        "9/17/26, 3:33 PM"
    );
}

#[test]
fn file_rows_show_size_date_and_folder() {
    let root = temporary_directory("details");
    let folder = root.join("rmac");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("AGENTS.md");
    std::fs::write(&file, vec![b'x'; 926]).unwrap();
    let provider = FileProvider::new(
        root.clone(),
        FakePaths {
            paths: vec![file.clone()],
        },
    );
    let results = provider.search("agents", &Cancellation::default()).unwrap();
    let subtitle = results[0].subtitle.clone().unwrap();
    assert!(subtitle.starts_with("926 bytes · Today, "), "{subtitle}");
    assert!(subtitle.ends_with(" · rmac"), "{subtitle}");
    std::fs::remove_dir_all(root).unwrap();
}

struct FakePaths {
    paths: Vec<PathBuf>,
}

impl FileSearch for FakePaths {
    fn filenames(
        &self,
        _: &Path,
        _: &str,
        _: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error> {
        Ok(self.paths.clone())
    }

    fn recents(&self, _: rmac_search::Options<'_>) -> Result<Vec<PathBuf>, rmac_search::Error> {
        Ok(self.paths.clone())
    }
}

#[test]
fn search_in_files_closes_every_query() {
    let results = SearchInFilesProvider
        .search(" quarterly report ", &Cancellation::default())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Search in Files");
    assert_eq!(results[0].category, Category::SearchIn);
    assert_eq!(
        results[0].primary,
        Action::SearchFiles {
            query: "quarterly report".into()
        }
    );
    assert!(SearchInFilesProvider
        .search("  ", &Cancellation::default())
        .unwrap()
        .is_empty());
}

#[test]
fn every_provider_has_its_own_stable_identity() {
    let descriptors = [
        ApplicationProvider::default().descriptor(),
        SettingsProvider::default().descriptor(),
        FileProvider::system(PathBuf::from("/home/alex")).descriptor(),
        CalculatorProvider.descriptor(),
        CurrencyProvider::new(FixedRates {
            rates: None,
            calls: AtomicUsize::new(0),
        })
        .descriptor(),
        WorldClockProvider::new(FakeClock).descriptor(),
        DictionaryProvider::new(Vec::new()).descriptor(),
        SearchInFilesProvider.descriptor(),
    ];
    let unique: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| (descriptor.id.clone(), descriptor.category))
        .collect();
    assert_eq!(unique.len(), descriptors.len());
}

fn temporary_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-launcher-answers-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock follows epoch")
            .as_nanos()
    ))
}
