/// Desktop words whose spelling follows the message locale: the owner's
/// en-GB Mac says "Bin", "Favourites", "Minimise" and "Centre".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileVocabulary {
    favourites: &'static str,
    bin: &'static str,
    minimise: &'static str,
    centre: &'static str,
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
                minimise: "Minimise",
                centre: "Centre",
            }
        } else {
            Self {
                favourites: "Favorites",
                bin: "Trash",
                minimise: "Minimize",
                centre: "Center",
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

    /// Window ▸ Minimise / Minimize.
    pub const fn minimise(self) -> &'static str {
        self.minimise
    }

    /// Window ▸ Centre / Center.
    pub const fn centre(self) -> &'static str {
        self.centre
    }
}

#[cfg(test)]
mod tests {
    use super::FileVocabulary;

    #[test]
    fn window_words_follow_the_locale() {
        let british = FileVocabulary::for_locale("en_GB.UTF-8");
        assert_eq!(
            (british.minimise(), british.centre()),
            ("Minimise", "Centre")
        );
        let american = FileVocabulary::for_locale("en_US.UTF-8");
        assert_eq!(
            (american.minimise(), american.centre()),
            ("Minimize", "Center")
        );
    }
}
