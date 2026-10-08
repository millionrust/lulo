/// Desktop words whose spelling follows the message locale: the owner's
/// en-GB Mac says "Bin", "Favourites", "Minimise" and "Centre". On Windows
/// a few Mac words become the ones Explorer uses (see [`WINDOWS_WORDS`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileVocabulary {
    favourites: &'static str,
    bin: &'static str,
    minimise: &'static str,
    centre: &'static str,
    windows: bool,
}

/// The one table of Mac words Lulo shows in Windows' own terms (ADR 0023):
/// each Mac menu label on the left reads as Explorer's on the right. The bin
/// itself is [`WINDOWS_BIN`], which every "Trash" label is built from
/// ([`FileVocabulary::bin`]): Move to Recycle Bin, Empty Recycle Bin…
pub const WINDOWS_WORDS: &[(&str, &str)] = &[
    ("Make Alias", "Create Shortcut"),
    ("Show Original", "Open File Location"),
    ("Copy as Pathname", "Copy as Path"),
    ("Computer", "This PC"),
];

/// The bin's name on Windows.
pub const WINDOWS_BIN: &str = "Recycle Bin";

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
                windows: false,
            }
        } else {
            Self {
                favourites: "Favorites",
                bin: "Trash",
                minimise: "Minimize",
                centre: "Center",
                windows: false,
            }
        }
    }

    /// The words for `locale`, in Windows' terms when `windows` is set.
    pub fn for_locale_on(locale: &str, windows: bool) -> Self {
        let mut words = Self::for_locale(locale);
        if windows {
            words.bin = WINDOWS_BIN;
            words.windows = true;
        }
        words
    }

    pub fn from_environment() -> Self {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "C".into());
        Self::for_locale_on(&locale, cfg!(windows))
    }

    /// Whether these are Windows' words.
    pub const fn is_windows(self) -> bool {
        self.windows
    }

    /// `mac` as this platform says it: unchanged, except on Windows where
    /// [`WINDOWS_WORDS`] gives Explorer's term.
    pub fn label(self, mac: &str) -> &str {
        if !self.windows {
            return mac;
        }
        WINDOWS_WORDS
            .iter()
            .find(|(from, _)| *from == mac)
            .map_or(mac, |(_, to)| to)
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
    use super::{FileVocabulary, WINDOWS_WORDS};

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

    #[test]
    fn windows_words_come_from_one_table() {
        for locale in ["en_GB.UTF-8", "en_US.UTF-8"] {
            let mac = FileVocabulary::for_locale(locale);
            let windows = FileVocabulary::for_locale_on(locale, true);
            assert!(!mac.is_windows());
            assert!(windows.is_windows());
            assert_eq!(FileVocabulary::for_locale_on(locale, false), mac);
            assert_eq!(windows.bin(), "Recycle Bin");
            assert_eq!(windows.favourites(), mac.favourites());
            assert_eq!(windows.label("Make Alias"), "Create Shortcut");
            assert_eq!(windows.label("Show Original"), "Open File Location");
            assert_eq!(windows.label("Copy as Pathname"), "Copy as Path");
            assert_eq!(windows.label("Computer"), "This PC");
            assert_eq!(windows.label("Duplicate"), "Duplicate");
            for (from, _) in WINDOWS_WORDS {
                assert_eq!(mac.label(from), *from, "unchanged off Windows");
            }
        }
    }
}
