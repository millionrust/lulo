//! A deterministic stand-in for the model, for behaviour scenarios and CI,
//! which have no model file. Selected only by
//! `RMAC_INTELLIGENCE_ENGINE=fixture`, and still subject to the user's
//! on/off setting. It answers a handful of fixed phrasings and "none" for
//! everything else, through the same strict intent parser as the model.

use std::time::Instant;

use rmac_intelligence::{prompt, Intent};

use crate::engine::{Calibration, Engine, EngineError, IntentOutcome, LoadReport, Timing};

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

impl Engine for FixtureEngine {
    fn intent(&mut self, text: &str, received: Instant) -> Result<IntentOutcome, EngineError> {
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
}
