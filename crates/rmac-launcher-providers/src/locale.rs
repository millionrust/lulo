//! Number and date wording for answers and file rows, following the
//! locale the way macOS follows the Region setting.
//!
//! Measured on the owner's Mac (English, India region, 2026-09-23):
//! `1000*1000` → "10,00,000", `2^10` → "1,024", `sqrt(2)` →
//! "1.4142135624", file dates "17/09/26, 3:33 PM" and "Today, 9:08 PM".

use std::time::{SystemTime, UNIX_EPOCH};

/// How the integer part is grouped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Grouping {
    /// 1,000,000
    Thousands,
    /// 10,00,000: the last three digits, then pairs (India).
    Indian,
}

/// Day/month order in short dates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateOrder {
    /// 17/09/26
    DayMonth,
    /// 9/17/26
    MonthDay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Locale {
    pub grouping: Grouping,
    pub decimal: char,
    pub group: char,
    pub date_order: DateOrder,
}

impl Default for Locale {
    fn default() -> Self {
        Self::from_tag("en_GB")
    }
}

impl Locale {
    /// From a POSIX locale name such as `en_IN.UTF-8` or `de_DE@euro`.
    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.split(['.', '@']).next().unwrap_or_default();
        let mut parts = tag.split(['_', '-']);
        let language = parts.next().unwrap_or_default().to_ascii_lowercase();
        let region = parts.next().unwrap_or_default().to_ascii_uppercase();
        let (decimal, group) = match language.as_str() {
            "de" | "es" | "it" | "nl" | "da" | "id" | "tr" | "el" | "ro" | "hr" | "sl" | "sr" => {
                (',', '.')
            }
            "pt" if region == "BR" => (',', '.'),
            "fr" | "ru" | "pl" | "sv" | "fi" | "nb" | "nn" | "no" | "cs" | "sk" | "uk" | "hu"
            | "bg" | "lt" | "lv" | "et" | "pt" => (',', '\u{202f}'),
            _ => ('.', ','),
        };
        let grouping = if region == "IN" {
            Grouping::Indian
        } else {
            Grouping::Thousands
        };
        let date_order = if matches!(region.as_str(), "US" | "PH" | "FM" | "MH" | "PW") {
            DateOrder::MonthDay
        } else {
            DateOrder::DayMonth
        };
        Self {
            grouping,
            decimal,
            group,
            date_order,
        }
    }

    /// The session's numeric locale: `LC_ALL`, then `LC_NUMERIC`, then
    /// `LANG`. `C` and `POSIX` read as British English.
    pub fn from_environment() -> Self {
        let tag = ["LC_ALL", "LC_NUMERIC", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        if tag.is_empty() || tag == "C" || tag.starts_with("C.") || tag == "POSIX" {
            Self::default()
        } else {
            Self::from_tag(&tag)
        }
    }

    /// `value` with at most `max_fraction` decimals, trailing zeros dropped
    /// and the integer part grouped. Very large or very small magnitudes use
    /// scientific notation ("1.2676506002e30"). `None` for NaN and infinity.
    pub fn format(&self, value: f64, max_fraction: usize) -> Option<String> {
        if !value.is_finite() {
            return None;
        }
        let magnitude = value.abs();
        if magnitude >= 1e15 || (magnitude != 0.0 && magnitude < 1e-10) {
            let text = format!("{value:.max_fraction$e}");
            let (mantissa, exponent) = text.split_once('e')?;
            let mantissa = trim_fraction(mantissa);
            return Some(format!(
                "{}e{exponent}",
                mantissa.replace('.', &self.decimal.to_string())
            ));
        }
        let text = format!("{value:.max_fraction$}");
        let text = trim_fraction(&text);
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.as_str()),
        };
        let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        let mut out = String::new();
        if negative && (integer != "0" || !fraction.is_empty()) {
            out.push('-');
        }
        out.push_str(&self.group_integer(integer));
        if !fraction.is_empty() {
            out.push(self.decimal);
            out.push_str(fraction);
        }
        Some(out)
    }

    fn group_integer(&self, integer: &str) -> String {
        let digits = integer.chars().collect::<Vec<_>>();
        if digits.len() <= 3 {
            return integer.to_owned();
        }
        // Group sizes from the right.
        let mut sizes = vec![3];
        let rest = digits.len() - 3;
        let step = match self.grouping {
            Grouping::Thousands => 3,
            Grouping::Indian => 2,
        };
        let mut remaining = rest;
        while remaining > 0 {
            let size = remaining.min(step);
            sizes.push(size);
            remaining -= size;
        }
        let mut groups = Vec::new();
        let mut end = digits.len();
        for size in sizes {
            groups.push(digits[end - size..end].iter().collect::<String>());
            end -= size;
        }
        groups.reverse();
        groups.join(&self.group.to_string())
    }

    /// "17/09/26" or "9/17/26".
    pub fn short_date(&self, year: i32, month: u32, day: u32) -> String {
        let year = year.rem_euclid(100);
        match self.date_order {
            DateOrder::DayMonth => format!("{day:02}/{month:02}/{year:02}"),
            DateOrder::MonthDay => format!("{month}/{day}/{year:02}"),
        }
    }
}

fn trim_fraction(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text.to_owned()
    }
}

/// "3:33 PM", as the Mac's menu bar and World Clock write times.
pub fn time_12h(hour: u32, minute: u32) -> String {
    let (display, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{display}:{minute:02} {suffix}")
}

/// Month abbreviations as the Mac writes them in English ("23 Sep").
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
