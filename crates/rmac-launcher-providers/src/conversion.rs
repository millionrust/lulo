//! Unit conversions for the answer card: "5 km in miles" = 3.11 miles.
//!
//! Measured on macOS 26.2 (see design-lab/spotlight.html): results keep two
//! decimals ("3.11 miles", "37.78 °C", "96.56 km/h", "4,046.86 m²"), masses
//! three ("11.023 lb"), and whole numbers drop their decimals ("1,000 MB",
//! "180 minutes"). The target is written the way it was typed: a word
//! ("miles", "minutes") stays a word, a symbol ("lb", "mb", "c", "m2")
//! becomes the canonical symbol ("lb", "MB", "°C", "m²").

use crate::locale::Locale;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dimension {
    Length,
    Mass,
    Temperature,
    Volume,
    Speed,
    Area,
    Time,
    Data,
}

impl Dimension {
    fn decimals(self) -> usize {
        match self {
            // "5 kg in lb" = 11.023 lb.
            Self::Mass => 3,
            _ => 2,
        }
    }
}

#[derive(Debug)]
pub struct Unit {
    pub dimension: Dimension,
    /// Canonical symbol ("km", "°C", "MB").
    pub symbol: &'static str,
    /// Spelled-out names, singular then plural ("mile", "miles").
    pub singular: &'static str,
    pub plural: &'static str,
    /// Other accepted spellings, lower case.
    pub aliases: &'static [&'static str],
    /// Value in the dimension's base unit (m, kg, K, L, m/s, m², s, byte)
    /// is `value × factor + offset`.
    pub factor: f64,
    pub offset: f64,
}

const fn unit(
    dimension: Dimension,
    symbol: &'static str,
    singular: &'static str,
    plural: &'static str,
    aliases: &'static [&'static str],
    factor: f64,
) -> Unit {
    Unit {
        dimension,
        symbol,
        singular,
        plural,
        aliases,
        factor,
        offset: 0.0,
    }
}

use Dimension::*;

const INCH: f64 = 0.0254;
const FOOT: f64 = 0.3048;
const YARD: f64 = 0.9144;
const MILE: f64 = 1_609.344;
const POUND: f64 = 0.453_592_37;
const US_GALLON: f64 = 3.785_411_784;
const DAY: f64 = 86_400.0;

pub static UNITS: &[Unit] = &[
    // Length, in metres.
    unit(
        Length,
        "mm",
        "millimetre",
        "millimetres",
        &["millimeter", "millimeters"],
        0.001,
    ),
    unit(
        Length,
        "cm",
        "centimetre",
        "centimetres",
        &["centimeter", "centimeters"],
        0.01,
    ),
    unit(Length, "m", "metre", "metres", &["meter", "meters"], 1.0),
    unit(
        Length,
        "km",
        "kilometre",
        "kilometres",
        &["kilometer", "kilometers", "kms"],
        1_000.0,
    ),
    unit(Length, "in", "inch", "inches", &["\""], INCH),
    unit(Length, "ft", "foot", "feet", &["'"], FOOT),
    unit(Length, "yd", "yard", "yards", &["yds"], YARD),
    unit(Length, "mi", "mile", "miles", &[], MILE),
    unit(
        Length,
        "nmi",
        "nautical mile",
        "nautical miles",
        &[],
        1_852.0,
    ),
    // Mass, in kilograms.
    unit(
        Mass,
        "mg",
        "milligram",
        "milligrams",
        &["milligramme", "milligrammes"],
        1e-6,
    ),
    unit(
        Mass,
        "g",
        "gram",
        "grams",
        &["gramme", "grammes", "gm", "gms"],
        0.001,
    ),
    unit(
        Mass,
        "kg",
        "kilogram",
        "kilograms",
        &["kilo", "kilos", "kgs", "kilogramme", "kilogrammes"],
        1.0,
    ),
    unit(
        Mass,
        "t",
        "tonne",
        "tonnes",
        &["metric ton", "metric tons"],
        1_000.0,
    ),
    unit(Mass, "lb", "pound", "pounds", &["lbs"], POUND),
    unit(Mass, "oz", "ounce", "ounces", &[], POUND / 16.0),
    unit(Mass, "st", "stone", "stone", &["stones"], POUND * 14.0),
    // Temperature, in kelvins (see `TEMPERATURES`).
    // Volume, in litres.
    unit(
        Volume,
        "mL",
        "millilitre",
        "millilitres",
        &["ml", "milliliter", "milliliters"],
        0.001,
    ),
    unit(
        Volume,
        "cL",
        "centilitre",
        "centilitres",
        &["cl", "centiliter", "centiliters"],
        0.01,
    ),
    unit(
        Volume,
        "L",
        "litre",
        "litres",
        &["l", "liter", "liters", "ltr"],
        1.0,
    ),
    unit(
        Volume,
        "m³",
        "cubic metre",
        "cubic metres",
        &["m3", "cubic meter", "cubic meters"],
        1_000.0,
    ),
    unit(
        Volume,
        "gal",
        "gallon",
        "gallons",
        &["us gallon", "us gallons"],
        US_GALLON,
    ),
    unit(
        Volume,
        "gal",
        "imperial gallon",
        "imperial gallons",
        &["uk gallon", "uk gallons"],
        4.546_09,
    ),
    unit(Volume, "qt", "quart", "quarts", &[], US_GALLON / 4.0),
    unit(Volume, "pt", "pint", "pints", &[], US_GALLON / 8.0),
    unit(Volume, "cup", "cup", "cups", &[], US_GALLON / 16.0),
    unit(
        Volume,
        "fl oz",
        "fluid ounce",
        "fluid ounces",
        &["floz", "fl. oz"],
        US_GALLON / 128.0,
    ),
    unit(
        Volume,
        "tbsp",
        "tablespoon",
        "tablespoons",
        &[],
        US_GALLON / 256.0,
    ),
    unit(
        Volume,
        "tsp",
        "teaspoon",
        "teaspoons",
        &[],
        US_GALLON / 768.0,
    ),
    // Speed, in metres per second.
    unit(
        Speed,
        "m/s",
        "metre per second",
        "metres per second",
        &["mps", "meters per second", "meter per second"],
        1.0,
    ),
    unit(
        Speed,
        "km/h",
        "kilometre per hour",
        "kilometres per hour",
        &[
            "kmh",
            "kph",
            "kmph",
            "kilometers per hour",
            "kilometer per hour",
        ],
        1_000.0 / 3_600.0,
    ),
    unit(
        Speed,
        "mph",
        "mile per hour",
        "miles per hour",
        &["mi/h"],
        MILE / 3_600.0,
    ),
    unit(
        Speed,
        "kn",
        "knot",
        "knots",
        &["kt", "kts"],
        1_852.0 / 3_600.0,
    ),
    unit(
        Speed,
        "ft/s",
        "foot per second",
        "feet per second",
        &["fps"],
        FOOT,
    ),
    // Area, in square metres.
    unit(
        Area,
        "mm²",
        "square millimetre",
        "square millimetres",
        &["mm2", "sq mm", "square millimeter", "square millimeters"],
        1e-6,
    ),
    unit(
        Area,
        "cm²",
        "square centimetre",
        "square centimetres",
        &["cm2", "sq cm", "square centimeter", "square centimeters"],
        1e-4,
    ),
    unit(
        Area,
        "m²",
        "square metre",
        "square metres",
        &["m2", "sq m", "sqm", "square meter", "square meters"],
        1.0,
    ),
    unit(
        Area,
        "km²",
        "square kilometre",
        "square kilometres",
        &["km2", "sq km", "square kilometer", "square kilometers"],
        1e6,
    ),
    unit(Area, "ha", "hectare", "hectares", &[], 10_000.0),
    unit(Area, "ac", "acre", "acres", &[], 4_046.856_422_4),
    unit(
        Area,
        "in²",
        "square inch",
        "square inches",
        &["in2", "sq in"],
        INCH * INCH,
    ),
    unit(
        Area,
        "ft²",
        "square foot",
        "square feet",
        &["ft2", "sq ft", "sqft"],
        FOOT * FOOT,
    ),
    unit(
        Area,
        "yd²",
        "square yard",
        "square yards",
        &["yd2", "sq yd"],
        YARD * YARD,
    ),
    unit(
        Area,
        "mi²",
        "square mile",
        "square miles",
        &["mi2", "sq mi"],
        MILE * MILE,
    ),
    // Time, in seconds.
    unit(Time, "ms", "millisecond", "milliseconds", &["msec"], 0.001),
    unit(Time, "s", "second", "seconds", &["sec", "secs"], 1.0),
    unit(Time, "min", "minute", "minutes", &["mins"], 60.0),
    unit(Time, "h", "hour", "hours", &["hr", "hrs"], 3_600.0),
    unit(Time, "d", "day", "days", &[], DAY),
    unit(Time, "wk", "week", "weeks", &["wks"], 7.0 * DAY),
    unit(Time, "mo", "month", "months", &[], 30.436_875 * DAY),
    unit(Time, "yr", "year", "years", &["yrs", "y"], 365.242_5 * DAY),
    // Data, in bytes. Letters are read as bytes ("1 gb in mb" = 1,000 MB).
    unit(Data, "bit", "bit", "bits", &[], 0.125),
    unit(Data, "B", "byte", "bytes", &["b"], 1.0),
    unit(Data, "KB", "kilobyte", "kilobytes", &["kb"], 1e3),
    unit(Data, "MB", "megabyte", "megabytes", &["mb"], 1e6),
    unit(Data, "GB", "gigabyte", "gigabytes", &["gb"], 1e9),
    unit(Data, "TB", "terabyte", "terabytes", &["tb"], 1e12),
    unit(Data, "PB", "petabyte", "petabytes", &["pb"], 1e15),
    unit(Data, "KiB", "kibibyte", "kibibytes", &["kib"], 1_024.0),
    unit(Data, "MiB", "mebibyte", "mebibytes", &["mib"], 1_048_576.0),
    unit(
        Data,
        "GiB",
        "gibibyte",
        "gibibytes",
        &["gib"],
        1_073_741_824.0,
    ),
    unit(
        Data,
        "TiB",
        "tebibyte",
        "tebibytes",
        &["tib"],
        1_099_511_627_776.0,
    ),
];

/// Temperatures have offsets, so they are listed on their own. The Mac
/// writes °C and °F whichever way they were typed.
pub static TEMPERATURES: &[Unit] = &[
    Unit {
        dimension: Temperature,
        symbol: "°C",
        singular: "°C",
        plural: "°C",
        aliases: &[
            "c",
            "celsius",
            "centigrade",
            "degrees celsius",
            "degree celsius",
            "deg c",
        ],
        factor: 1.0,
        offset: 273.15,
    },
    Unit {
        dimension: Temperature,
        symbol: "°F",
        singular: "°F",
        plural: "°F",
        aliases: &[
            "f",
            "fahrenheit",
            "degrees fahrenheit",
            "degree fahrenheit",
            "deg f",
        ],
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
    },
    Unit {
        dimension: Temperature,
        symbol: "K",
        singular: "kelvin",
        plural: "kelvins",
        aliases: &["k"],
        factor: 1.0,
        offset: 0.0,
    },
];

/// Words that join the amount to the target ("in", "to", "as", "=", "→").
pub(crate) const CONNECTORS: &[&str] = &["in", "to", "into", "as", "=", "->", "→"];

/// A unit as it was written: whether the person spelled it out.
#[derive(Clone, Copy, Debug)]
pub struct Written {
    pub unit: &'static Unit,
    pub spelled: bool,
}

/// Look up a unit by any of its spellings. Symbols are matched
/// case-sensitively first ("mL", "B") and then case-insensitively.
pub fn lookup(text: &str) -> Option<Written> {
    let text = normalize_unit(text);
    if text.is_empty() {
        return None;
    }
    let all = || UNITS.iter().chain(TEMPERATURES.iter());
    if let Some(unit) = all().find(|unit| unit.symbol == text) {
        return Some(Written {
            unit,
            spelled: false,
        });
    }
    let lower = text.to_lowercase();
    for unit in all() {
        if unit.singular.to_lowercase() == lower || unit.plural.to_lowercase() == lower {
            return Some(Written {
                unit,
                spelled: unit.singular != unit.symbol,
            });
        }
    }
    for unit in all() {
        if unit.symbol.to_lowercase() == lower || unit.aliases.contains(&lower.as_str()) {
            // Spelled-out aliases ("meters", "kilos", "square meters") read
            // as words; abbreviations ("sq ft", "mins", "kmph") do not.
            let spelled = unit.dimension != Temperature
                && lower
                    .split(' ')
                    .all(|word| word.chars().count() > 4 && word.chars().all(char::is_alphabetic));
            return Some(Written { unit, spelled });
        }
    }
    None
}

fn normalize_unit(text: &str) -> String {
    let text = text
        .trim()
        .trim_end_matches('.')
        .replace('²', "2")
        .replace('³', "3")
        .replace('°', "");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The leading amount and the words after it: "5km to mi" → (5, ["km",
/// "to", "mi"]).
pub(crate) fn amount_and_words(query: &str) -> Option<(f64, Vec<&str>)> {
    let query = query.trim();
    let number_end = query
        .char_indices()
        .find(|(index, character)| {
            !(character.is_ascii_digit()
                || *character == '.'
                || *character == ','
                || (*index == 0 && (*character == '-' || *character == '+')))
        })
        .map_or(query.len(), |(index, _)| index);
    let amount = parse_amount(&query[..number_end])?;
    Some((amount, query[number_end..].split_whitespace().collect()))
}

/// Every way to read `words` as "source connector target", in order: "12
/// in in cm" gives ("in", "in cm") and then ("in in", "cm").
pub(crate) fn connector_splits(words: &[&str]) -> Vec<(String, String)> {
    words
        .iter()
        .enumerate()
        .filter(|(index, word)| {
            *index > 0
                && index + 1 < words.len()
                && CONNECTORS.contains(&word.to_lowercase().as_str())
        })
        .map(|(index, _)| (words[..index].join(" "), words[index + 1..].join(" ")))
        .collect()
}

/// "1,000" and "1000.5"; commas only as thousands separators.
pub(crate) fn parse_amount(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let plain = text.replace(',', "");
    if plain.is_empty() || plain == "-" || plain == "+" || plain == "." {
        return None;
    }
    let value: f64 = plain.parse().ok()?;
    value.is_finite().then_some(value)
}

/// "5 km in miles" → "3.11 miles".
pub fn convert(query: &str, locale: &Locale) -> Option<String> {
    let (amount, source, target) = candidates(query)?;
    if source.unit.dimension != target.unit.dimension || std::ptr::eq(source.unit, target.unit) {
        return None;
    }
    let base = amount * source.unit.factor + source.unit.offset;
    let value = (base - target.unit.offset) / target.unit.factor;
    let text = locale.format(value, source.unit.dimension.decimals())?;
    let label = if target.spelled {
        if text == "1" {
            target.unit.singular
        } else {
            target.unit.plural
        }
    } else {
        target.unit.symbol
    };
    Some(format!("{text} {label}"))
}

/// The first reading that names two units of one dimension: "12 in in cm"
/// reads as inches to centimetres.
fn candidates(query: &str) -> Option<(f64, Written, Written)> {
    let (amount, words) = amount_and_words(query)?;
    connector_splits(&words)
        .into_iter()
        .find_map(|(source, target)| {
            let (source, target) = (lookup(&source)?, lookup(&target)?);
            (source.unit.dimension == target.unit.dimension).then_some((source, target))
        })
        .map(|(source, target)| (amount, source, target))
}
