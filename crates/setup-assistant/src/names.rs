//! Human names for installed locales, account monograms, and the
//! Language & Region lists.

/// One installed locale the lists can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleChoice {
    /// The exact installed name, e.g. `en_GB.UTF-8`.
    pub code: String,
    /// "English (United Kingdom)".
    pub language: String,
    /// "United Kingdom".
    pub region: String,
}

/// Language names in their own language (how macOS lists them).
const LANGUAGES: [(&str, &str); 30] = [
    ("ar", "العربية"),
    ("bn", "বাংলা"),
    ("ca", "Català"),
    ("cs", "Čeština"),
    ("da", "Dansk"),
    ("de", "Deutsch"),
    ("el", "Ελληνικά"),
    ("en", "English"),
    ("es", "Español"),
    ("fi", "Suomi"),
    ("fr", "Français"),
    ("he", "עברית"),
    ("hi", "हिन्दी"),
    ("hu", "Magyar"),
    ("id", "Bahasa Indonesia"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("ml", "മലയാളം"),
    ("nb", "Norsk Bokmål"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("ro", "Română"),
    ("ru", "Русский"),
    ("sv", "Svenska"),
    ("ta", "தமிழ்"),
    ("tr", "Türkçe"),
    ("uk", "Українська"),
    ("zh", "中文"),
];

/// Region names (English; the lists follow the chosen language only after
/// the next login, when the session picks it up).
const REGIONS: [(&str, &str); 40] = [
    ("AE", "United Arab Emirates"),
    ("AR", "Argentina"),
    ("AT", "Austria"),
    ("AU", "Australia"),
    ("BE", "Belgium"),
    ("BR", "Brazil"),
    ("CA", "Canada"),
    ("CH", "Switzerland"),
    ("CN", "China mainland"),
    ("CZ", "Czechia"),
    ("DE", "Germany"),
    ("DK", "Denmark"),
    ("EG", "Egypt"),
    ("ES", "Spain"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("GB", "United Kingdom"),
    ("GR", "Greece"),
    ("HK", "Hong Kong"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IN", "India"),
    ("IT", "Italy"),
    ("JP", "Japan"),
    ("KR", "South Korea"),
    ("MX", "Mexico"),
    ("NL", "Netherlands"),
    ("NO", "Norway"),
    ("NZ", "New Zealand"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("RU", "Russia"),
    ("SA", "Saudi Arabia"),
    ("SE", "Sweden"),
    ("SG", "Singapore"),
    ("TR", "Türkiye"),
    ("TW", "Taiwan"),
    ("UA", "Ukraine"),
    ("US", "United States"),
    ("ZA", "South Africa"),
];

/// `(language, territory)` of a locale name such as `en_GB.UTF-8@euro`.
pub fn locale_parts(code: &str) -> Option<(&str, &str)> {
    let base = code.split(['.', '@']).next()?;
    let (language, territory) = base.split_once('_')?;
    (!language.is_empty() && !territory.is_empty()).then_some((language, territory))
}

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, name)| *name)
}

/// Describe one locale, or `None` for `C`, `POSIX` and other locales
/// without a territory.
pub fn describe(code: &str) -> Option<LocaleChoice> {
    let (language, territory) = locale_parts(code)?;
    let region = lookup(&REGIONS, territory)
        .map(str::to_owned)
        .unwrap_or_else(|| territory.to_owned());
    let language_name = lookup(&LANGUAGES, language)
        .map(str::to_owned)
        .unwrap_or_else(|| language.to_owned());
    Some(LocaleChoice {
        code: code.to_owned(),
        language: format!("{language_name} ({region})"),
        region,
    })
}

/// The installed locales as list entries: one per language/territory
/// (UTF-8 preferred), sorted by their language label.
pub fn choices(installed: &[String]) -> Vec<LocaleChoice> {
    let mut out: Vec<LocaleChoice> = Vec::new();
    for code in installed {
        let Some(choice) = describe(code) else {
            continue;
        };
        let utf8 = is_utf8(code);
        if let Some(existing) = out.iter_mut().find(|other| {
            locale_parts(&other.code) == locale_parts(code)
                && modifier(&other.code) == modifier(code)
        }) {
            if utf8 && !is_utf8(&existing.code) {
                *existing = choice;
            }
            continue;
        }
        out.push(choice);
    }
    out.sort_by(|a, b| a.language.cmp(&b.language).then(a.code.cmp(&b.code)));
    out
}

fn is_utf8(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    lower.contains(".utf-8") || lower.contains(".utf8")
}

fn modifier(code: &str) -> Option<&str> {
    code.split_once('@').map(|(_, modifier)| modifier)
}

/// Whether two locale names name the same locale (`en_GB.utf8` and
/// `en_GB.UTF-8` do).
pub fn same_locale(a: &str, b: &str) -> bool {
    locale_parts(a) == locale_parts(b) && modifier(a) == modifier(b)
}

/// The account picture's monogram: the first letter of the first and last
/// words of the full name, else of the login name.
pub fn monogram(real_name: &str, user_name: &str) -> String {
    let words = real_name.split_whitespace().collect::<Vec<_>>();
    let initial = |word: &str| {
        word.chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
    };
    match words.as_slice() {
        [] => user_name
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default(),
        [only] => initial(only).unwrap_or_default(),
        [first, .., last] => format!(
            "{}{}",
            initial(first).unwrap_or_default(),
            initial(last).unwrap_or_default()
        ),
    }
}
