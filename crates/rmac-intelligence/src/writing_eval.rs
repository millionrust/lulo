//! The Writing Tools evaluation (ADR 0024 "Phase 2 (Writing Tools)
//! results"): 30 original passages (`tests/intelligence/writing-passages.jsonl`)
//! and deterministic checks for each task.
//!
//! - **Proofread** inputs are made by injecting four known errors into each
//!   clean passage ([`inject_errors`]: confused words, swapped or dropped
//!   letters, a lower-case sentence start, a missing apostrophe), so the
//!   right answer is exact. [`score_proofread`] aligns the answer with the
//!   clean passage word by word and counts the errors fixed and the words
//!   changed that were already right.
//! - **Summarise** is checked for length (5–60 words, at most 60 % of the
//!   passage) and for keeping the passage's key terms.
//! - **Rewrite, Friendly, Professional, Concise** keep the key terms and
//!   every number, actually change the wording, and stay in their length
//!   band (Concise: at most 85 % of the passage).
//! - **Create Key Points** and **Make List** answer with 2–6 (Key Points)
//!   or 2+ (List) "• " lines that keep the key terms.
//!
//! The checks measure what can be measured without a judge model; the
//! phase 2 results also record answers read by hand.

use serde::Deserialize;

use crate::writing::BULLET;
use crate::Task;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Passage {
    pub id: String,
    pub text: String,
    /// Names, numbers and nouns any faithful answer keeps.
    pub terms: Vec<String>,
}

pub fn parse_passages(jsonl: &str) -> Result<Vec<Passage>, serde_json::Error> {
    jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

/// One injected error: the clean passage's word `index` (by whitespace)
/// was replaced with `wrong`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injected {
    pub index: usize,
    pub right: String,
    pub wrong: String,
}

/// Commonly confused pairs: (right, wrong).
const CONFUSIONS: &[(&str, &str)] = &[
    ("their", "there"),
    ("there", "their"),
    ("your", "you're"),
    ("its", "it's"),
    ("it's", "its"),
    ("than", "then"),
    ("then", "than"),
    ("to", "too"),
    ("too", "to"),
    ("were", "where"),
    ("where", "were"),
    ("whether", "weather"),
    ("buy", "by"),
    ("know", "no"),
    ("passed", "past"),
    ("affect", "effect"),
    ("lose", "loose"),
    ("which", "witch"),
];

/// A small deterministic generator, so the set is the same everywhere.
struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Self {
        // FNV-1a.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in seed.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        Self(hash | 1)
    }

    fn next(&mut self) -> u64 {
        // xorshift64*.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }
}

/// Split a word into (leading punctuation, core, trailing punctuation).
fn split_word(word: &str) -> (&str, &str, &str) {
    let start = word
        .char_indices()
        .find(|(_, character)| character.is_alphanumeric())
        .map_or(word.len(), |(index, _)| index);
    let end = word
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric())
        .map_or(start, |(index, character)| index + character.len_utf8());
    (
        &word[..start],
        &word[start..end.max(start)],
        &word[end.max(start)..],
    )
}

fn is_term_word(word: &str, terms: &[String]) -> bool {
    let (_, core, _) = split_word(word);
    let core = core.to_lowercase();
    !core.is_empty()
        && terms.iter().any(|term| {
            term.to_lowercase()
                .split_whitespace()
                .any(|part| part == core)
        })
}

/// The ways one word can be broken. `None` when this kind does not apply.
fn corrupt(kind: usize, word: &str, sentence_start: bool, rng: &mut Rng) -> Option<String> {
    let (lead, core, trail) = split_word(word);
    if core.is_empty() || core.chars().any(|character| character.is_ascii_digit()) {
        return None;
    }
    let lower = core.to_lowercase();
    let wrong_core = match kind {
        // A confused word.
        0 => CONFUSIONS
            .iter()
            .find(|(right, _)| *right == lower)
            .map(|(_, wrong)| {
                if core.chars().next().is_some_and(char::is_uppercase) {
                    let mut characters = wrong.chars();
                    characters
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                        .unwrap_or_default()
                } else {
                    (*wrong).to_owned()
                }
            })?,
        // Two neighbouring letters swapped, inside a longer word.
        1 => {
            let characters: Vec<char> = core.chars().collect();
            if characters.len() < 5 || !characters.iter().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            let at = 1 + rng.below(characters.len() - 3);
            if characters[at] == characters[at + 1] {
                return None;
            }
            let mut swapped = characters.clone();
            swapped.swap(at, at + 1);
            swapped.into_iter().collect()
        }
        // One letter dropped (a doubled letter if there is one).
        2 => {
            let characters: Vec<char> = core.chars().collect();
            if characters.len() < 5 || !characters.iter().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            let doubled =
                (1..characters.len()).find(|&index| characters[index] == characters[index - 1]);
            let at = doubled.unwrap_or_else(|| 1 + rng.below(characters.len() - 2));
            let mut dropped = characters.clone();
            dropped.remove(at);
            dropped.into_iter().collect()
        }
        // A sentence that starts in lower case.
        3 => {
            if !sentence_start
                || !core.chars().next().is_some_and(char::is_uppercase)
                || core == "I"
            {
                return None;
            }
            let mut characters = core.chars();
            characters
                .next()
                .map(|first| first.to_lowercase().collect::<String>() + characters.as_str())?
        }
        // An apostrophe left out.
        _ => {
            if !core.contains('\'') && !core.contains('\u{2019}') {
                return None;
            }
            core.replace(['\'', '\u{2019}'], "")
        }
    };
    (wrong_core != core).then(|| format!("{lead}{wrong_core}{trail}"))
}

/// Inject `count` errors into `passage`, at least two words apart and never
/// in a key term. Returns the broken text and the errors, in word order.
pub fn inject_errors(passage: &Passage, count: usize) -> (String, Vec<Injected>) {
    let words: Vec<&str> = passage.text.split_whitespace().collect();
    let mut rng = Rng::new(&passage.id);
    let mut injected: Vec<Injected> = Vec::new();
    let mut attempts = 0;
    // Every kind gets a turn before any repeats.
    let mut kind = rng.below(5);
    while injected.len() < count && attempts < 4000 {
        attempts += 1;
        let index = rng.below(words.len());
        if injected.iter().any(|error| error.index.abs_diff(index) < 2)
            || is_term_word(words[index], &passage.terms)
        {
            continue;
        }
        let sentence_start = index == 0 || words[index - 1].ends_with(['.', '!', '?']);
        if let Some(wrong) = corrupt(kind, words[index], sentence_start, &mut rng) {
            injected.push(Injected {
                index,
                right: words[index].to_owned(),
                wrong,
            });
            kind = (kind + 1) % 5;
        } else if attempts % 40 == 0 {
            kind = (kind + 1) % 5;
        }
    }
    injected.sort_by_key(|error| error.index);
    let broken = words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            injected
                .iter()
                .find(|error| error.index == index)
                .map_or(*word, |error| error.wrong.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ");
    (broken, injected)
}

fn normalise(word: &str) -> String {
    word.replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
}

/// For each word of `left`, whether a longest common subsequence with
/// `right` keeps it; and that subsequence's length.
fn lcs_matches(left: &[String], right: &[String]) -> (Vec<bool>, usize) {
    let (n, m) = (left.len(), right.len());
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if left[i] == right[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut kept = vec![false; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if left[i] == right[j] {
            kept[i] = true;
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (kept, table[0][0] as usize)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofreadScore {
    pub injected: u32,
    pub fixed: u32,
    /// Words that were already right but changed, added or dropped.
    pub unwanted_changes: u32,
    /// The answer is the clean passage exactly (up to apostrophe style).
    pub exact: bool,
}

/// Score a proofread answer against the clean passage it should restore.
pub fn score_proofread(clean: &str, injected: &[Injected], answer: &str) -> ProofreadScore {
    let target: Vec<String> = clean.split_whitespace().map(normalise).collect();
    let output: Vec<String> = answer.split_whitespace().map(normalise).collect();
    let (kept, common) = lcs_matches(&target, &output);
    let fixed = injected
        .iter()
        .filter(|error| kept.get(error.index).copied().unwrap_or(false))
        .count() as u32;
    let missed = injected.len() - fixed as usize;
    let changed = (target.len() - common).max(output.len() - common);
    ProofreadScore {
        injected: injected.len() as u32,
        fixed,
        unwanted_changes: changed.saturating_sub(missed) as u32,
        exact: target == output,
    }
}

fn words(text: &str) -> u32 {
    text.split_whitespace().count() as u32
}

/// The share of `terms` the answer still names (case-insensitive).
pub fn term_coverage(terms: &[String], answer: &str) -> f32 {
    if terms.is_empty() {
        return 1.0;
    }
    let answer = normalise(answer).to_lowercase();
    terms
        .iter()
        .filter(|term| answer.contains(&normalise(term).to_lowercase()))
        .count() as f32
        / terms.len() as f32
}

/// Every number in the passage (any word with a digit, without its
/// punctuation) is in the answer too.
pub fn numbers_kept(passage: &str, answer: &str) -> bool {
    passage
        .split_whitespace()
        .map(|word| split_word(word).1)
        .filter(|core| core.chars().any(|character| character.is_ascii_digit()))
        .all(|number| answer.contains(number))
}

/// The digits of every number-like word, without punctuation or units
/// ("8.30" and "8:30" read the same; "£12" and "12" too).
fn number_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !(character.is_ascii_digit() || matches!(character, '.' | ':')))
        .map(|token| token.trim_matches(|character: char| !character.is_ascii_digit()))
        .filter(|token| token.chars().any(|character| character.is_ascii_digit()))
        .map(|token| token.replace(':', "."))
        .collect()
}

/// Numbers the answer has that the passage does not (each answer number is
/// looked for among the passage's, so a kept "17" in "17 June" is fine).
/// Number words in the passage count: "two" allows "2", "ten" "10".
pub fn invented_numbers(passage: &str, answer: &str) -> Vec<String> {
    const WORDS: [(&str, &str); 13] = [
        ("one", "1"),
        ("two", "2"),
        ("three", "3"),
        ("four", "4"),
        ("five", "5"),
        ("six", "6"),
        ("seven", "7"),
        ("eight", "8"),
        ("nine", "9"),
        ("ten", "10"),
        ("eleven", "11"),
        ("twelve", "12"),
        ("twenty", "20"),
    ];
    let lower = passage.to_lowercase();
    let mut known = number_tokens(passage);
    for (word, digits) in WORDS {
        if lower
            .split(|c: char| !c.is_alphabetic())
            .any(|token| token == word)
        {
            known.push(digits.to_owned());
        }
    }
    if lower.contains("thirty-two") {
        known.push("32".into());
    }
    if lower.contains("midnight") {
        known.push("12".into());
    }
    let mut invented = Vec::new();
    for number in number_tokens(answer) {
        let whole = number.split('.').next().unwrap_or_default().to_owned();
        let found = known
            .iter()
            .any(|known| known == &number || known.split('.').next() == Some(whole.as_str()));
        if !found && !invented.contains(&number) {
            invented.push(number);
        }
    }
    invented
}

fn has_preface(answer: &str) -> bool {
    let first = answer
        .trim_start()
        .lines()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    first.starts_with("here is")
        || first.starts_with("here's")
        || first.starts_with("sure")
        || first.starts_with("certainly")
}

fn bullet_lines(answer: &str) -> (usize, bool) {
    let lines: Vec<&str> = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let all = !lines.is_empty() && lines.iter().all(|line| line.starts_with(BULLET.trim_end()));
    (lines.len(), all)
}

/// One task's verdict on one passage, with the measured values behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub pass: bool,
    pub words: u32,
    /// Answer words over passage words.
    pub ratio: f32,
    pub terms: f32,
    pub numbers_kept: bool,
    /// Numbers in the answer that the passage never had: a made-up date,
    /// time or amount, the worst mistake a writing tool can make.
    pub invented: Vec<String>,
    pub preface: bool,
    /// Bulleted lines (Key Points, List).
    pub bullets: usize,
    /// Why it failed, for the report.
    pub reason: String,
}

/// Check one non-proofread answer for `passage`.
pub fn check(task: Task, passage: &Passage, answer: &str) -> Check {
    let input_words = words(&passage.text).max(1);
    let answer_words = words(answer);
    let ratio = answer_words as f32 / input_words as f32;
    let terms = term_coverage(&passage.terms, answer);
    let numbers = numbers_kept(&passage.text, answer);
    let invented = invented_numbers(&passage.text, answer);
    let preface = has_preface(answer);
    let (bullets, all_bulleted) = bullet_lines(answer);
    let changed = {
        let left: Vec<String> = passage.text.split_whitespace().map(normalise).collect();
        let right: Vec<String> = answer.split_whitespace().map(normalise).collect();
        let (_, common) = lcs_matches(&left, &right);
        (common as f32) < 0.9 * left.len().max(right.len()) as f32
    };
    let mut reasons = Vec::new();
    let mut need = |ok: bool, why: &str| {
        if !ok {
            reasons.push(why.to_owned());
        }
    };
    need(!preface, "preface");
    need(invented.is_empty(), "no invented numbers");
    need(answer_words > 0, "empty");
    match task {
        Task::Summarise => {
            need((5..=60).contains(&answer_words), "length 5-60 words");
            need(ratio <= 0.6, "at most 60% of the passage");
            need(terms >= 0.5, "half the key terms");
        }
        Task::Rewrite | Task::Friendly | Task::Professional => {
            need((0.6..=1.8).contains(&ratio), "length 60-180%");
            need(terms >= 0.75, "three quarters of the key terms");
            need(numbers, "every number");
            need(changed, "new wording");
        }
        Task::Concise => {
            need((0.25..=0.85).contains(&ratio), "length 25-85%");
            need(terms >= 0.5, "half the key terms");
            need(numbers || terms >= 0.75, "the numbers");
        }
        Task::KeyPoints => {
            need(
                (2..=6).contains(&bullets) && all_bulleted,
                "2-6 bullet lines",
            );
            need(ratio <= 1.0, "shorter than the passage");
            need(terms >= 0.5, "half the key terms");
        }
        Task::List => {
            need(bullets >= 2 && all_bulleted, "2+ bullet lines");
            need(terms >= 0.6, "most key terms");
        }
        Task::Proofread | Task::Intent => need(false, "not checked here"),
    }
    Check {
        pass: reasons.is_empty(),
        words: answer_words,
        ratio,
        terms,
        numbers_kept: numbers,
        invented,
        preface,
        bullets,
        reason: reasons.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSAGES: &str = include_str!("../../../tests/intelligence/writing-passages.jsonl");

    #[test]
    fn the_set_is_thirty_original_passages_with_their_terms() {
        let passages = parse_passages(PASSAGES).unwrap();
        assert_eq!(passages.len(), 30);
        let ids: std::collections::BTreeSet<_> = passages.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), 30);
        for passage in &passages {
            assert!(passage.terms.len() >= 3, "{}", passage.id);
            assert!(
                term_coverage(&passage.terms, &passage.text) == 1.0,
                "{}",
                passage.id
            );
        }
        // The speed passage is a 150-word rewrite.
        let timing = passages.iter().find(|p| p.id == "p30-rewrite-150").unwrap();
        assert_eq!(words(&timing.text), 150);
    }

    #[test]
    fn every_passage_gets_four_known_errors() {
        for passage in parse_passages(PASSAGES).unwrap() {
            let (broken, injected) = inject_errors(&passage, 4);
            assert_eq!(injected.len(), 4, "{}", passage.id);
            assert_ne!(broken, passage.text);
            for error in &injected {
                assert_ne!(error.right, error.wrong);
                assert!(!is_term_word(&error.right, &passage.terms));
            }
            // The clean passage scores perfectly; the broken one fixes nothing.
            let perfect = score_proofread(&passage.text, &injected, &passage.text);
            assert_eq!(
                (perfect.fixed, perfect.unwanted_changes, perfect.exact),
                (4, 0, true)
            );
            let untouched = score_proofread(&passage.text, &injected, &broken);
            assert_eq!(untouched.fixed, 0, "{}", passage.id);
            assert_eq!(untouched.unwanted_changes, 0, "{}", passage.id);
            // Deterministic.
            assert_eq!(inject_errors(&passage, 4), (broken, injected));
        }
    }

    #[test]
    fn proofread_scores_count_fixes_and_damage() {
        let clean = "They went to the market and bought apples.";
        let injected = vec![
            Injected {
                index: 2,
                right: "to".into(),
                wrong: "too".into(),
            },
            Injected {
                index: 6,
                right: "bought".into(),
                wrong: "bougth".into(),
            },
        ];
        // One fixed, one missed, one correct word changed.
        let score = score_proofread(clean, &injected, "They went to a market and bougth apples.");
        assert_eq!(score.fixed, 1);
        assert_eq!(score.unwanted_changes, 1);
        assert!(!score.exact);
        // Curly apostrophes count as the same word.
        assert!(score_proofread("it's fine", &[], "it\u{2019}s fine").exact);
    }

    #[test]
    fn made_up_numbers_are_caught() {
        let passage = "The coach leaves at 8.30 on 7 March and costs 12 pounds; two teachers come.";
        assert!(invented_numbers(passage, "Leaves 8:30, 7 March, £12, 2 teachers.").is_empty());
        assert_eq!(invented_numbers(passage, "Leaves at 9 on 7 March."), ["9"]);
        assert_eq!(
            invented_numbers(passage, "Costs 15 pounds in 2025."),
            ["15", "2025"]
        );
    }

    #[test]
    fn checks_follow_each_task() {
        let passage = Passage {
            id: "t".into(),
            text: "Omar will fix the payment bug by 17 June, and Grace will rewrite the help pages for the launch next month after legal has checked them.".into(),
            terms: vec!["Omar".into(), "Grace".into(), "17 June".into(), "payment".into()],
        };
        let summary = check(
            Task::Summarise,
            &passage,
            "Omar fixes the payment bug by 17 June; Grace rewrites the help.",
        );
        assert!(summary.pass, "{summary:?}");
        let too_long = check(Task::Summarise, &passage, &passage.text);
        assert!(!too_long.pass);
        let preface = check(Task::Rewrite, &passage, "Here is the text:\nOmar will repair the payment fault before 17 June, while Grace rewrites the help pages for next month's launch once legal approves them.");
        assert!(!preface.pass && preface.preface);
        let rewrite = check(Task::Rewrite, &passage, "Omar will repair the payment fault before 17 June, while Grace rewrites the help pages for next month's launch once legal approves them.");
        assert!(rewrite.pass, "{rewrite:?}");
        let copy = check(Task::Friendly, &passage, &passage.text);
        assert!(!copy.pass, "an unchanged copy is not a rewrite");
        let dropped_number = check(Task::Professional, &passage, "Omar will repair the payment fault soon, while Grace rewrites the help pages for next month's launch once legal approves them.");
        assert!(!dropped_number.pass);
        let points = check(
            Task::KeyPoints,
            &passage,
            "• Omar fixes the payment bug by 17 June.\n• Grace rewrites the help pages.",
        );
        assert!(points.pass, "{points:?}");
        let prose = check(
            Task::KeyPoints,
            &passage,
            "Omar fixes the payment bug by 17 June.",
        );
        assert!(!prose.pass);
        let list = check(
            Task::List,
            &passage,
            "• Omar: fix the payment bug by 17 June\n• Grace: rewrite the help pages",
        );
        assert!(list.pass, "{list:?}");
        let concise = check(
            Task::Concise,
            &passage,
            "Omar fixes the payment bug by 17 June; Grace rewrites the help pages after legal.",
        );
        assert!(concise.pass, "{concise:?}");
    }
}
