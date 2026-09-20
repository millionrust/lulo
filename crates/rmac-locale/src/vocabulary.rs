/// File-management words whose spelling follows the message locale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileVocabulary {
    favourites: &'static str,
    bin: &'static str,
}

impl FileVocabulary {
    pub fn for_locale(locale: &str) -> Self {
        let locale = locale
            .split(['.', '@'])
            .next()
            .unwrap_or(locale)
            .replace('-', "_");
        if locale.eq_ignore_ascii_case("en_GB") {
            Self {
                favourites: "Favourites",
                bin: "Bin",
            }
        } else {
            Self {
                favourites: "Favorites",
                bin: "Trash",
            }
        }
    }

    pub fn from_environment() -> Self {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "C".into());
        Self::for_locale(&locale)
    }

    pub const fn favourites(self) -> &'static str {
        self.favourites
    }

    pub const fn bin(self) -> &'static str {
        self.bin
    }
}
