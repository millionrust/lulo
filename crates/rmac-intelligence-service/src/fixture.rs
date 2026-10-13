//! A deterministic stand-in for the model, for behaviour scenarios and CI,
//! which have no model file. Selected only by
//! `RMAC_INTELLIGENCE_ENGINE=fixture`, and still subject to the user's
//! on/off setting. It answers a handful of fixed phrasings and "none" for
//! everything else, through the same strict intent parser as the model.
//!
//! For Writing Tools it writes a fixed transformation of the text (see
//! [`write_answer`]) one word at a time, `RMAC_INTELLIGENCE_FIXTURE_DELAY_MS`
//! (default 80 ms) apart, so a behaviour scenario can watch the answer
//! stream in and cancel it halfway. `RMAC_INTELLIGENCE_FIXTURE_TIER=tiny`
//! makes it a Tiny-tier engine, for the "Needs a faster PC" gate.

use std::time::{Duration, Instant};

use rmac_intelligence::manifest::Tier;
use rmac_intelligence::{prompt, writing, Intent, Task};

use crate::engine::{
    Calibration, Engine, EngineError, IntentOutcome, LoadReport, Timing, WriteOutcome,
    WriteRequest, WriteTiming,
};

#[derive(Default)]
pub struct FixtureEngine;

pub fn answer(text: &str) -> &'static str {
    let text = prompt::clean_request(text).to_lowercase();
    let has = |word: &str| text.split(' ').any(|token| token == word);
    if text.contains("dark mode") {
        r#"{"intent":"appearance","mode":"dark"}"#
    } else if text.contains("light mode") {
        r#"{"intent":"appearance","mode":"light"}"#
    } else if has("open") && has("notes") {
        r#"{"intent":"open_app","app":"Notes"}"#
    } else if has("volume") && has("30%") {
        r#"{"intent":"volume","level":30}"#
    } else if has("timer") && has("10") && has("minutes") {
        r#"{"intent":"timer","amount":10,"unit":"minutes"}"#
    } else if has("wifi") && has("off") {
        r#"{"intent":"wifi","on":false}"#
    } else {
        r#"{"intent":"none"}"#
    }
}

/// Misspellings the fixture's Proofread corrects.
const TYPOS: &[(&str, &str)] = &[
    ("teh", "the"),
    ("recieve", "receive"),
    ("dont", "don't"),
    ("i", "I"),
    ("wierd", "weird"),
    ("seperate", "separate"),
    ("tommorow", "tomorrow"),
    ("definately", "definitely"),
];

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        current.push(character);
        if matches!(character, '.' | '!' | '?') {
            let sentence = current.trim().to_owned();
            if !sentence.is_empty() {
                out.push(sentence);
            }
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

fn proofread(text: &str) -> String {
    let mut start = true;
    text.split(' ')
        .map(|word| {
            let core = word.trim_end_matches(['.', ',', '!', '?', ';', ':']);
            let tail = &word[core.len()..];
            let fixed = TYPOS
                .iter()
                .find(|(wrong, _)| *wrong == core)
                .map_or(core.to_owned(), |(_, right)| (*right).to_owned());
            let fixed = if start {
                let mut characters = fixed.chars();
                characters
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                    .unwrap_or_default()
            } else {
                fixed
            };
            start = word.ends_with(['.', '!', '?']);
            format!("{fixed}{tail}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The fixture's answer: a fixed, recognisable stand-in for each task.
pub fn write_answer(task: Task, text: &str, attempt: u32) -> String {
    let fixed = proofread(text);
    let all = sentences(&fixed);
    let first = all.first().cloned().unwrap_or_default();
    let bullets = |items: &[String]| {
        items
            .iter()
            .map(|sentence| format!("{}{sentence}", writing::BULLET))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let answer = match task {
        Task::Proofread | Task::Intent => fixed,
        Task::Rewrite => all.iter().rev().cloned().collect::<Vec<_>>().join(" "),
        Task::Friendly => format!("Hi! {fixed} Thanks so much!"),
        Task::Professional => format!("Hello, {fixed} Kind regards."),
        Task::Concise => first,
        Task::Summarise => first.split(' ').take(12).collect::<Vec<_>>().join(" "),
        Task::KeyPoints => bullets(&all[..all.len().min(3)]),
        Task::List => bullets(&all),
    };
    if attempt > 0 && task != Task::Proofread {
        format!("{answer} (Version {})", attempt + 1)
    } else {
        answer
    }
}

fn delay() -> Duration {
    Duration::from_millis(
        std::env::var("RMAC_INTELLIGENCE_FIXTURE_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(80)
            .min(2000),
    )
}

/// Split an answer into the words it streams, each with the whitespace
/// before it, so joining them gives the answer back exactly.
fn pieces(answer: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    for character in answer.chars() {
        if character.is_whitespace() && !current.trim().is_empty() {
            pieces.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

impl Engine for FixtureEngine {
    fn intent(
        &mut self,
        text: &str,
        received: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<IntentOutcome, EngineError> {
        if cancelled() {
            return Err(EngineError::Cancelled);
        }
        let json = answer(text);
        let intent = Intent::parse(json).map_err(|error| EngineError::Failed(error.to_string()))?;
        let elapsed = received.elapsed().as_secs_f64() * 1000.0;
        Ok(IntentOutcome {
            intent,
            json: json.to_owned(),
            timing: Timing {
                first_token_ms: elapsed,
                total_ms: elapsed,
                ..Timing::default()
            },
        })
    }

    fn write(
        &mut self,
        request: &WriteRequest<'_>,
        sink: &mut dyn FnMut(&str),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<WriteOutcome, EngineError> {
        let spec = writing::spec(request.task)
            .ok_or_else(|| EngineError::Failed("not a writing task".into()))?;
        let input_tokens = writing::word_count(request.text) * 4 / 3;
        if input_tokens > spec.max_input_tokens {
            return Err(EngineError::TooLong);
        }
        if !spec.offered_on(self.tier()) {
            return Err(EngineError::NeedsFasterPc);
        }
        let answer = write_answer(request.task, request.text, request.attempt);
        let pause = delay();
        let mut raw = String::new();
        let mut first_token_ms = 0.0;
        let mut output_tokens = 0;
        for piece in pieces(&answer) {
            if cancelled() {
                return Err(EngineError::Cancelled);
            }
            std::thread::sleep(pause);
            if output_tokens == 0 {
                first_token_ms = request.received.elapsed().as_secs_f64() * 1000.0;
            }
            sink(&piece);
            raw.push_str(&piece);
            output_tokens += 1;
        }
        let total_ms = request.received.elapsed().as_secs_f64() * 1000.0;
        Ok(WriteOutcome {
            text: writing::finish(request.task, request.text, &raw),
            raw,
            capped: false,
            timing: WriteTiming {
                first_token_ms,
                total_ms,
                input_tokens,
                output_tokens,
                ..WriteTiming::default()
            },
        })
    }

    fn calibrate(&mut self) -> Result<Calibration, EngineError> {
        Ok(Calibration {
            tier: "fixture".into(),
            decode_tok_s: 0.0,
            prefill_tok_s: 0.0,
        })
    }

    fn load_report(&self) -> LoadReport {
        LoadReport::default()
    }

    fn tier(&self) -> Tier {
        match std::env::var("RMAC_INTELLIGENCE_FIXTURE_TIER").as_deref() {
            Ok("tiny") => Tier::Tiny,
            _ => Tier::Standard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_answers_are_valid_intents() {
        for text in [
            "dark mode on",
            "turn on light mode",
            "open Notes",
            "volume 30%",
            "set a timer for 10 minutes",
            "wifi off",
            "what is the weather",
        ] {
            assert!(Intent::parse(answer(text)).is_ok(), "{text}");
        }
        assert_eq!(
            Intent::parse(answer("dark mode on"))
                .unwrap()
                .title(None)
                .as_deref(),
            Some("Turn On Dark Mode")
        );
        assert_eq!(
            Intent::parse(answer("tell me a joke")).unwrap(),
            Intent::None
        );
    }

    #[test]
    fn fixture_writing_answers_are_recognisable() {
        assert_eq!(
            write_answer(Task::Proofread, "i think teh cat is wierd. it sat", 0),
            "I think the cat is weird. It sat"
        );
        assert_eq!(write_answer(Task::Proofread, "teh end.", 3), "The end.");
        assert_eq!(write_answer(Task::Rewrite, "One. Two.", 0), "Two. One.");
        assert_eq!(
            write_answer(Task::Friendly, "Hello.", 1),
            "Hi! Hello. Thanks so much! (Version 2)"
        );
        assert_eq!(
            write_answer(Task::List, "Buy milk. Call Sam.", 0),
            "• Buy milk.\n• Call Sam."
        );
        assert_eq!(write_answer(Task::Concise, "First. Second.", 0), "First.");
    }

    #[test]
    fn streamed_pieces_join_back_into_the_answer() {
        for answer in ["Hi there.", "• One\n• Two", "  lead", ""] {
            assert_eq!(pieces(answer).concat(), answer);
        }
        assert_eq!(pieces("a b").len(), 2);
    }
}
