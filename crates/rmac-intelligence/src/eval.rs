//! The intent evaluation sets (`tests/intelligence/*.jsonl`) and scoring.
//!
//! Each line is `{"request": "...", "expect": <intent wire JSON>, "tags":
//! [...]}`. A case passes when the decoded intent is the expected one:
//! the same action and the same values, comparing app names and file
//! queries without case. A refusal case expects `{"intent":"none"}`; a
//! wrong action on one of those is the dangerous kind of miss, counted on
//! its own as `wrong_action`.

use serde::Deserialize;

use crate::Intent;

#[derive(Clone, Debug, Deserialize)]
struct Line {
    request: String,
    expect: serde_json::Value,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub request: String,
    pub expect: Intent,
    pub tags: Vec<String>,
}

pub fn parse_cases(jsonl: &str) -> Result<Vec<Case>, String> {
    jsonl
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(number, line)| {
            let line: Line = serde_json::from_str(line)
                .map_err(|error| format!("line {}: {error}", number + 1))?;
            let expect = Intent::from_value(&line.expect)
                .map_err(|error| format!("line {}: {error}", number + 1))?;
            Ok(Case {
                request: line.request,
                expect,
                tags: line.tags,
            })
        })
        .collect()
}

pub fn matches(expected: &Intent, got: &Intent) -> bool {
    match (expected, got) {
        (Intent::OpenApp { app: left }, Intent::OpenApp { app: right }) => same_text(left, right),
        (Intent::SearchFiles { query: left }, Intent::SearchFiles { query: right }) => {
            same_text(left, right)
        }
        // "1 hour" and "60 minutes" start the same timer.
        (Intent::Timer { .. }, Intent::Timer { .. }) => {
            expected.timer_seconds() == got.timer_seconds()
        }
        _ => expected == got,
    }
}

/// The model's text holds every word of the expected text ("budget" in
/// "budget files", "Settings" in "System Settings"), ignoring case and
/// quotes, and adds at most three words.
fn same_text(expected: &str, got: &str) -> bool {
    let words = |text: &str| {
        text.to_lowercase()
            .split(|character: char| character.is_whitespace() || matches!(character, '\'' | '"'))
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let expected = words(expected);
    let got = words(got);
    !expected.is_empty()
        && expected.iter().all(|word| got.contains(word))
        && got.len() <= expected.len() + 3
}

/// Totals for a run over one set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Score {
    pub cases: usize,
    pub correct: usize,
    /// An action where none was right, or a different action than the one
    /// asked for: the misses that would change the wrong thing.
    pub wrong_action: usize,
    /// Asked for an action, got "none": harmless, just unhelpful.
    pub missed: usize,
}

impl Score {
    pub fn add(&mut self, expected: &Intent, got: &Intent) {
        self.cases += 1;
        if matches(expected, got) {
            self.correct += 1;
        } else if *got == Intent::None {
            self.missed += 1;
        } else {
            self.wrong_action += 1;
        }
    }

    pub fn percent(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            self.correct as f64 * 100.0 / self.cases as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const DEV: &str = include_str!("../../../tests/intelligence/intents-dev.jsonl");
    const HELD_OUT: &str = include_str!("../../../tests/intelligence/intents-heldout.jsonl");
    /// The held-out set is frozen: it is never used to tune the prompt.
    /// Changing it needs a new hash here and a note in ADR 0024.
    const HELD_OUT_SHA256: &str =
        include_str!("../../../tests/intelligence/intents-heldout.sha256");

    #[test]
    fn the_sets_parse_and_cover_every_intent_and_tag() {
        let dev = parse_cases(DEV).unwrap();
        let held_out = parse_cases(HELD_OUT).unwrap();
        assert!(
            dev.len() + held_out.len() >= 60,
            "{}",
            dev.len() + held_out.len()
        );
        assert!(dev.len() >= 60, "the extended set has {} cases", dev.len());
        assert!(held_out.len() >= 30);
        for cases in [&dev, &held_out] {
            let names: std::collections::BTreeSet<_> =
                cases.iter().map(|case| case.expect.name()).collect();
            assert_eq!(names.len(), crate::INTENT_NAMES.len());
            for tag in ["typo", "indian-english", "refusal"] {
                assert!(
                    cases.iter().any(|case| case.tags.iter().any(|t| t == tag)),
                    "{tag}"
                );
            }
        }
        // No request is in two sets, or in the prompt's own examples.
        let mut seen = std::collections::BTreeSet::new();
        for case in dev.iter().chain(&held_out) {
            assert!(seen.insert(case.request.to_lowercase()), "{}", case.request);
        }
        for (request, _) in crate::prompt::EXAMPLES {
            assert!(!seen.contains(&request.to_lowercase()), "{request}");
        }
    }

    #[test]
    fn the_held_out_set_is_frozen() {
        let digest = crate::verify::hex(&Sha256::digest(HELD_OUT.as_bytes()));
        assert_eq!(digest, HELD_OUT_SHA256.trim());
    }

    #[test]
    fn scoring_separates_harmless_misses_from_wrong_actions() {
        let dark = Intent::parse(r#"{"intent":"appearance","mode":"dark"}"#).unwrap();
        let light = Intent::parse(r#"{"intent":"appearance","mode":"light"}"#).unwrap();
        let mut score = Score::default();
        score.add(&dark, &dark);
        score.add(&dark, &Intent::None);
        score.add(&dark, &light);
        score.add(&Intent::None, &dark);
        assert_eq!(
            score,
            Score {
                cases: 4,
                correct: 1,
                wrong_action: 2,
                missed: 1,
            }
        );
        assert!(matches(
            &Intent::OpenApp {
                app: "Notes".into()
            },
            &Intent::OpenApp {
                app: "notes".into()
            }
        ));
    }
}
