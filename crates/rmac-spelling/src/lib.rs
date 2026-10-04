//! A hunspell-dictionary-backed [`rmac_ui::text_assist::SpellChecker`] for
//! Edit ▸ Spelling and Grammar.
//!
//! Loads one of Ubuntu's existing `/usr/share/hunspell` dictionaries (no
//! bundled dictionary, matching the low-spec-PC budget) through the
//! pure-Rust `spellbook` crate, lazily and off the UI thread: the first
//! call to [`shared`] returns immediately and starts a background thread
//! that parses the dictionary; every check before that thread finishes
//! fails open (`is_correct` returns `None`, which callers treat as
//! "correct" rather than flashing false positives). Kept as its own crate,
//! depended on only by the four apps with a Spelling and Grammar menu
//! (Clock, Notes, Preview, Text Editor), so `rmac-ui` and the shell
//! binaries that depend on it never pull in `spellbook`.

use std::sync::{Arc, Mutex, OnceLock};

use rmac_ui::text_assist::SpellChecker;

enum LoadState {
    NotStarted,
    Loading,
    Ready(Box<spellbook::Dictionary>),
    /// No dictionary found (not Ubuntu, or `hunspell-en-gb`/`-en-us` not
    /// installed) or it failed to parse.
    Unavailable,
}

pub struct HunspellChecker {
    state: Mutex<LoadState>,
}

impl HunspellChecker {
    fn new() -> Self {
        Self {
            state: Mutex::new(LoadState::NotStarted),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LoadState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn start_loading(self: &Arc<Self>) {
        {
            let mut state = self.lock();
            if !matches!(*state, LoadState::NotStarted) {
                return;
            }
            *state = LoadState::Loading;
        }
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let loaded = load_dictionary();
            let mut state = this.lock();
            *state = match loaded {
                Some(dictionary) => LoadState::Ready(Box::new(dictionary)),
                None => LoadState::Unavailable,
            };
        });
    }
}

impl SpellChecker for HunspellChecker {
    fn is_correct(&self, word: &str) -> Option<bool> {
        match &*self.lock() {
            LoadState::Ready(dictionary) => Some(dictionary.check(word)),
            LoadState::NotStarted | LoadState::Loading | LoadState::Unavailable => None,
        }
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        match &*self.lock() {
            LoadState::Ready(dictionary) => {
                let mut suggestions = Vec::new();
                dictionary.suggest(word, &mut suggestions);
                suggestions
            }
            LoadState::NotStarted | LoadState::Loading | LoadState::Unavailable => Vec::new(),
        }
    }
}

/// The dictionaries tried, in order, matching the project's en-GB default
/// with an en-US fallback for a system that only has the latter installed.
const CANDIDATE_DICTIONARIES: &[&str] = &["en_GB", "en_US"];
const DICTIONARY_DIR: &str = "/usr/share/hunspell";

fn load_dictionary() -> Option<spellbook::Dictionary> {
    for name in CANDIDATE_DICTIONARIES {
        let base = std::path::Path::new(DICTIONARY_DIR).join(name);
        let aff_path = base.with_extension("aff");
        let dic_path = base.with_extension("dic");
        let (Ok(aff), Ok(dic)) = (
            std::fs::read_to_string(&aff_path),
            std::fs::read_to_string(&dic_path),
        ) else {
            continue;
        };
        match spellbook::Dictionary::new(&aff, &dic) {
            Ok(dictionary) => return Some(dictionary),
            Err(error) => eprintln!("rmac-spelling: could not parse {name}: {error}"),
        }
    }
    None
}

/// The one checker this process uses, started lazily: the first call
/// kicks off the background load and every call (the first included)
/// returns right away.
pub fn shared() -> Arc<HunspellChecker> {
    static CHECKER: OnceLock<Arc<HunspellChecker>> = OnceLock::new();
    let checker = CHECKER.get_or_init(|| Arc::new(HunspellChecker::new()));
    checker.start_loading();
    Arc::clone(checker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checker_that_has_not_finished_loading_fails_open() {
        let checker = HunspellChecker::new();
        assert_eq!(checker.is_correct("anything"), None);
        assert_eq!(checker.suggest("anything"), Vec::<String>::new());
    }

    #[test]
    fn shared_checker_eventually_reports_a_definite_answer_when_a_dictionary_is_installed() {
        if !std::path::Path::new(DICTIONARY_DIR).exists() {
            // Not Ubuntu (this Mac, CI's macOS job): nothing to assert.
            return;
        }
        let checker = shared();
        for _ in 0..200 {
            if checker.is_correct("the").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Some(correct) = checker.is_correct("the") {
            assert!(correct);
            assert_eq!(checker.is_correct("zzxxqqzzxx"), Some(false));
        }
        // Else: hunspell-en-gb/-en-us is simply not installed here either;
        // fail-open is already covered above.
    }
}
