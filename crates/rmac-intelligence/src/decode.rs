//! Schema-guided decoding: the model can only write a valid intent.
//!
//! This is grammar-constrained decoding written against the intent schema
//! itself rather than a general GBNF engine. At every step the decoder
//! knows exactly which continuations the schema allows and reads only those
//! tokens' logits:
//!
//! - **choices** (an intent name, `dark`/`light`, `true`/`false`, a unit):
//!   each alternative is tokenized once, together with the literal JSON that
//!   follows it, and the decoder walks the alternatives' token sequences,
//!   keeping those consistent with the best-scoring next token. As soon as
//!   one alternative is left, the rest of it is *forced*;
//! - **numbers**: single-digit tokens, with the range enforced digit by
//!   digit, then the following literal;
//! - **free text** (an app name, a file query): any token whose text holds
//!   no quote, backslash or control character, up to a length cap, then the
//!   closing quote.
//!
//! Forced tokens are never decoded one at a time: they are queued and fed
//! to the model in one batch the next time a logit is needed, and the
//! closing `}` is never fed at all. "turn on dark mode" costs the prompt's
//! request tokens plus two forward passes.
//!
//! Because the decoder only ever emits text the schema allows, the result
//! always parses ([`Intent::parse`]); a unit test drives it with random
//! logits to hold that.

use std::fmt;

use crate::prompt::ANSWER_PREFIX;
use crate::{Intent, IntentError, MAX_TEXT_BYTES};

pub type Token = i32;

/// The model's tokenizer, as the decoder needs it.
pub trait Vocabulary {
    /// Tokenize plain text (no special tokens).
    fn tokenize(&self, text: &str) -> Vec<Token>;
    /// The bytes a token stands for.
    fn piece(&self, token: Token) -> &[u8];
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The running model: feed tokens after everything so far, get the logits
/// for the token after the last one.
pub trait Model {
    fn feed(&mut self, tokens: &[Token]) -> Result<Vec<f32>, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    Model(String),
    /// The vocabulary cannot express a schema literal (never for a real
    /// tokenizer, which can spell any text).
    Vocabulary,
    Intent(IntentError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Model(detail) => write!(formatter, "the model failed: {detail}"),
            Self::Vocabulary => formatter.write_str("the vocabulary cannot spell the schema"),
            Self::Intent(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Precomputed facts about a vocabulary, built once per model load.
pub struct Tables {
    /// Tokens that may appear inside a JSON string the decoder writes.
    text_safe: Vec<bool>,
    /// The single-digit tokens "0"–"9".
    digits: [Option<Token>; 10],
}

impl Tables {
    pub fn new(vocabulary: &impl Vocabulary) -> Self {
        let mut digits = [None; 10];
        let mut text_safe = vec![false; vocabulary.len()];
        for (index, safe) in text_safe.iter_mut().enumerate() {
            let token = index as Token;
            let piece = vocabulary.piece(token);
            if let [digit @ b'0'..=b'9'] = piece {
                let slot = &mut digits[usize::from(digit - b'0')];
                if slot.is_none() {
                    *slot = Some(token);
                }
            }
            *safe = !piece.is_empty()
                && std::str::from_utf8(piece).is_ok_and(|text| {
                    !text
                        .chars()
                        .any(|character| matches!(character, '"' | '\\') || character.is_control())
                });
        }
        Self { text_safe, digits }
    }
}

/// What one decode produced, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub intent: Intent,
    /// The full JSON the model wrote, including [`ANSWER_PREFIX`].
    pub json: String,
    /// Forward passes after the prompt (each may carry several tokens).
    pub passes: u32,
    /// Tokens fed after the prompt.
    pub tokens: u32,
}

/// Continue an answer whose prompt ended with [`ANSWER_PREFIX`].
/// `logits` are the model's logits after that prompt.
pub fn decode(
    model: &mut impl Model,
    vocabulary: &impl Vocabulary,
    tables: &Tables,
    logits: Vec<f32>,
) -> Result<Decoded, DecodeError> {
    let mut run = Run {
        model,
        vocabulary,
        tables,
        logits,
        pending: Vec::new(),
        json: ANSWER_PREFIX.to_owned(),
        passes: 0,
        tokens: 0,
    };
    let names = [
        "open_app\",\"app\":\"",
        "appearance\",\"mode\":\"",
        "volume\",\"",
        "brightness\",\"",
        "wifi\",\"on\":",
        "bluetooth\",\"on\":",
        "do_not_disturb\",\"on\":",
        "timer\",\"amount\":",
        "search_files\",\"query\":\"",
        "none\"}",
    ];
    match run.choose(&names)? {
        0 | 8 => run.text()?,
        1 => {
            run.choose(&["dark\"}", "light\"}"])?;
        }
        2 => {
            if run.choose(&["level\":", "change\":\""])? == 0 {
                run.number(0, 100, "}")?;
            } else {
                run.choose(&["up\"}", "down\"}", "mute\"}", "unmute\"}"])?;
            }
        }
        3 => {
            if run.choose(&["level\":", "change\":\""])? == 0 {
                run.number(0, 100, "}")?;
            } else {
                run.choose(&["up\"}", "down\"}"])?;
            }
        }
        4..=6 => {
            run.choose(&["true}", "false}"])?;
        }
        7 => {
            // Clock's limit is just under a day: no more than 23 hours.
            if run.number(1, 999, ",\"unit\":\"")? <= 23 {
                run.choose(&["seconds\"}", "minutes\"}", "hours\"}"])?;
            } else {
                run.choose(&["seconds\"}", "minutes\"}"])?;
            }
        }
        _ => {}
    }
    let intent = Intent::parse(&run.json).map_err(DecodeError::Intent)?;
    Ok(Decoded {
        intent,
        json: run.json,
        passes: run.passes,
        tokens: run.tokens,
    })
}

struct Run<'a, M, V> {
    model: &'a mut M,
    vocabulary: &'a V,
    tables: &'a Tables,
    logits: Vec<f32>,
    /// Chosen or forced tokens not yet fed to the model.
    pending: Vec<Token>,
    json: String,
    passes: u32,
    tokens: u32,
}

impl<M: Model, V: Vocabulary> Run<'_, M, V> {
    fn logit(&self, token: Token) -> f32 {
        usize::try_from(token)
            .ok()
            .and_then(|index| self.logits.get(index))
            .copied()
            .unwrap_or(f32::NEG_INFINITY)
    }

    /// Make `logits` describe the position after every pending token.
    fn flush(&mut self) -> Result<(), DecodeError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let tokens = std::mem::take(&mut self.pending);
        self.logits = self.model.feed(&tokens).map_err(DecodeError::Model)?;
        self.passes += 1;
        self.tokens += tokens.len() as u32;
        Ok(())
    }

    fn force(&mut self, literal: &str) -> Result<(), DecodeError> {
        let tokens = self.vocabulary.tokenize(literal);
        if tokens.is_empty() && !literal.is_empty() {
            return Err(DecodeError::Vocabulary);
        }
        self.pending.extend(tokens);
        self.json.push_str(literal);
        Ok(())
    }

    /// Pick one of `alternatives` (each the full literal up to the next free
    /// slot). Returns its index.
    fn choose(&mut self, alternatives: &[&str]) -> Result<usize, DecodeError> {
        let spelled: Vec<Vec<Token>> = alternatives
            .iter()
            .map(|alternative| self.vocabulary.tokenize(alternative))
            .collect();
        if spelled.iter().any(Vec::is_empty) {
            return Err(DecodeError::Vocabulary);
        }
        let mut alive: Vec<usize> = (0..alternatives.len()).collect();
        let mut position = 0;
        loop {
            if let [only] = alive[..] {
                self.pending.extend_from_slice(&spelled[only][position..]);
                self.json.push_str(alternatives[only]);
                return Ok(only);
            }
            // Alternatives that are a token prefix of another end here; the
            // schema never has those (every alternative carries its own
            // closing literal), but stay total: the shortest one wins.
            if let Some(&ended) = alive
                .iter()
                .find(|&&index| spelled[index].len() == position)
            {
                self.json.push_str(alternatives[ended]);
                return Ok(ended);
            }
            self.flush()?;
            let best = alive
                .iter()
                .map(|&index| spelled[index][position])
                .max_by(|left, right| self.logit(*left).total_cmp(&self.logit(*right)))
                .ok_or(DecodeError::Vocabulary)?;
            alive.retain(|&index| spelled[index][position] == best);
            self.pending.push(best);
            position += 1;
        }
    }

    /// An integer from `minimum` to `maximum`, then `follower`.
    fn number(&mut self, minimum: u32, maximum: u32, follower: &str) -> Result<u32, DecodeError> {
        let follower_tokens = self.vocabulary.tokenize(follower);
        let end = *follower_tokens.first().ok_or(DecodeError::Vocabulary)?;
        let mut value: Option<u32> = None;
        loop {
            self.flush()?;
            let mut best: Option<(f32, Option<u32>, Token)> = None;
            for (digit, token) in self.tables.digits.iter().enumerate() {
                let Some(token) = *token else { continue };
                let digit = digit as u32;
                let next = match value {
                    // No leading zeros: "0" stays alone.
                    Some(0) => continue,
                    Some(current) => current * 10 + digit,
                    None if digit == 0 && minimum > 0 => continue,
                    None => digit,
                };
                if next > maximum {
                    continue;
                }
                let score = self.logit(token);
                if best.is_none_or(|(top, _, _)| score > top) {
                    best = Some((score, Some(digit), token));
                }
            }
            if value.is_some_and(|value| value >= minimum) {
                let score = self.logit(end);
                if best.is_none_or(|(top, _, _)| score > top) {
                    best = Some((score, None, end));
                }
            }
            match best {
                Some((_, Some(digit), token)) => {
                    let next = value.map_or(digit, |current| current * 10 + digit);
                    value = Some(next);
                    self.json.push_str(&digit.to_string());
                    self.pending.push(token);
                    // No longer number fits: the follower is forced.
                    if next * 10 > maximum {
                        self.force(follower)?;
                        return Ok(next);
                    }
                }
                Some((_, None, _)) => {
                    self.force(follower)?;
                    return value.ok_or(DecodeError::Vocabulary);
                }
                None => return Err(DecodeError::Vocabulary),
            }
        }
    }

    /// A JSON string's contents, then `"}`.
    fn text(&mut self) -> Result<(), DecodeError> {
        let close_all = self.vocabulary.tokenize("\"}");
        let close_quote = self.vocabulary.tokenize("\"");
        let ends = [close_all.first().copied(), close_quote.first().copied()];
        let mut written = String::new();
        loop {
            self.flush()?;
            let mut best: Option<(f32, Token)> = None;
            for (index, score) in self.logits.iter().enumerate() {
                let token = index as Token;
                let ending = ends.contains(&Some(token));
                let allowed = if ending {
                    !written.trim().is_empty()
                } else {
                    self.tables.text_safe.get(index).copied().unwrap_or(false)
                        && piece_fits(&written, self.vocabulary.piece(token))
                };
                if allowed && best.is_none_or(|(top, _)| *score > top) {
                    best = Some((*score, token));
                }
            }
            let Some((_, token)) = best else {
                // Nothing fits any more: close the string.
                return self.close_text(&written);
            };
            if ends.contains(&Some(token)) {
                return self.close_text(&written);
            }
            let piece = String::from_utf8_lossy(self.vocabulary.piece(token)).into_owned();
            // A leading space before the first word is not part of a name.
            let piece = if written.is_empty() {
                piece.trim_start().to_owned()
            } else {
                piece
            };
            written.push_str(&piece);
            self.json.push_str(&piece);
            self.pending.push(token);
            if written.len() >= MAX_TEXT_BYTES {
                return self.close_text(&written);
            }
        }
    }

    fn close_text(&mut self, written: &str) -> Result<(), DecodeError> {
        if written.trim().is_empty() {
            return Err(DecodeError::Intent(IntentError::InvalidValue("text")));
        }
        // Trailing spaces would be trimmed by the parser anyway; keep the
        // wire text identical to what parses.
        let trimmed = written.trim_end().len();
        let cut = written.len() - trimmed;
        self.json.truncate(self.json.len() - cut);
        self.force("\"}")
    }
}

/// Whether appending `piece` keeps the text within the cap and starts it
/// with something other than whitespace.
fn piece_fits(written: &str, piece: &[u8]) -> bool {
    if written.len() + piece.len() > MAX_TEXT_BYTES {
        return false;
    }
    !(written.is_empty() && piece.iter().all(u8::is_ascii_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy byte-level vocabulary: every ASCII printable character is a
    /// token, plus a few multi-character tokens, tokenized greedily.
    struct Toy {
        pieces: Vec<Vec<u8>>,
    }

    impl Toy {
        fn new() -> Self {
            let mut pieces: Vec<Vec<u8>> = (0x20u8..0x7f).map(|byte| vec![byte]).collect();
            for extra in [
                "\"}",
                "open",
                "_app",
                "\",\"",
                "dark",
                "light",
                "true",
                "false",
                "volume",
                "level",
                "Notes",
                " Notes",
                "none",
                "minutes",
                "\n",
                "<|im_end|>",
            ] {
                pieces.push(extra.as_bytes().to_vec());
            }
            Self { pieces }
        }

        fn token(&self, text: &str) -> Token {
            self.pieces
                .iter()
                .position(|piece| piece == text.as_bytes())
                .unwrap() as Token
        }
    }

    impl Vocabulary for Toy {
        fn tokenize(&self, text: &str) -> Vec<Token> {
            let bytes = text.as_bytes();
            let mut tokens = Vec::new();
            let mut at = 0;
            while at < bytes.len() {
                let (index, piece) = self
                    .pieces
                    .iter()
                    .enumerate()
                    .filter(|(_, piece)| bytes[at..].starts_with(piece))
                    .max_by_key(|(_, piece)| piece.len())
                    .unwrap();
                tokens.push(index as Token);
                at += piece.len();
            }
            tokens
        }

        fn piece(&self, token: Token) -> &[u8] {
            &self.pieces[token as usize]
        }

        fn len(&self) -> usize {
            self.pieces.len()
        }
    }

    /// Prefers the next token of `target` (its tokenization after the
    /// answer prefix) whenever that token is allowed.
    struct Scripted<'a> {
        vocabulary: &'a Toy,
        target: Vec<Token>,
        fed: usize,
        passes: usize,
    }

    impl Scripted<'_> {
        fn logits(&self) -> Vec<f32> {
            let mut logits = vec![0.0; self.vocabulary.len()];
            if let Some(next) = self.target.get(self.fed) {
                logits[*next as usize] = 10.0;
            }
            logits
        }
    }

    impl Model for Scripted<'_> {
        fn feed(&mut self, tokens: &[Token]) -> Result<Vec<f32>, String> {
            self.fed += tokens.len();
            self.passes += 1;
            Ok(self.logits())
        }
    }

    fn decode_towards(target_json: &str) -> Decoded {
        let toy = Toy::new();
        let tables = Tables::new(&toy);
        let rest = target_json.strip_prefix(ANSWER_PREFIX).unwrap();
        let mut model = Scripted {
            vocabulary: &toy,
            target: toy.tokenize(rest),
            fed: 0,
            passes: 0,
        };
        let logits = model.logits();
        decode(&mut model, &toy, &tables, logits).unwrap()
    }

    #[test]
    fn a_willing_model_gets_exactly_what_it_means() {
        for json in [
            r#"{"intent":"appearance","mode":"dark"}"#,
            r#"{"intent":"appearance","mode":"light"}"#,
            r#"{"intent":"open_app","app":"Notes"}"#,
            r#"{"intent":"volume","level":30}"#,
            r#"{"intent":"volume","level":100}"#,
            r#"{"intent":"volume","level":0}"#,
            r#"{"intent":"volume","change":"unmute"}"#,
            r#"{"intent":"brightness","change":"up"}"#,
            r#"{"intent":"wifi","on":false}"#,
            r#"{"intent":"bluetooth","on":true}"#,
            r#"{"intent":"do_not_disturb","on":true}"#,
            r#"{"intent":"timer","amount":25,"unit":"minutes"}"#,
            r#"{"intent":"search_files","query":"tax 2025"}"#,
            r#"{"intent":"none"}"#,
        ] {
            let decoded = decode_towards(json);
            assert_eq!(decoded.json, json);
            assert_eq!(decoded.intent, Intent::parse(json).unwrap());
        }
    }

    #[test]
    fn forced_text_is_batched_and_the_close_is_never_fed() {
        // "appearance" is spelled one character per toy token, so the choice
        // is settled after its first token ("a" is unique among the names);
        // everything up to the mode is then fed in one pass, and the mode's
        // first token settles dark/light, with its close never fed.
        let decoded = decode_towards(r#"{"intent":"appearance","mode":"dark"}"#);
        assert_eq!(decoded.passes, 1);
    }

    /// A model that answers with random logits.
    struct Noise {
        state: u64,
        size: usize,
    }

    impl Noise {
        fn next(&mut self) -> f32 {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            (self.state % 10_000) as f32 / 100.0
        }

        fn logits(&mut self) -> Vec<f32> {
            (0..self.size).map(|_| self.next()).collect()
        }
    }

    impl Model for Noise {
        fn feed(&mut self, _tokens: &[Token]) -> Result<Vec<f32>, String> {
            Ok(self.logits())
        }
    }

    #[test]
    fn any_model_output_is_a_valid_intent() {
        let toy = Toy::new();
        let tables = Tables::new(&toy);
        let mut seen = std::collections::BTreeSet::new();
        for seed in 1..=3_000u64 {
            let mut model = Noise {
                state: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
                size: toy.len(),
            };
            let logits = model.logits();
            let decoded = decode(&mut model, &toy, &tables, logits)
                .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
            assert_eq!(Intent::parse(&decoded.json), Ok(decoded.intent.clone()));
            seen.insert(decoded.intent.name());
        }
        // Random logits reach every intent, so every branch was exercised.
        assert_eq!(seen.len(), crate::INTENT_NAMES.len());
    }

    #[test]
    fn digits_and_text_safety_come_from_the_vocabulary() {
        let toy = Toy::new();
        let tables = Tables::new(&toy);
        assert_eq!(tables.digits[7], Some(toy.token("7")));
        assert!(!tables.text_safe[toy.token("\"") as usize]);
        assert!(!tables.text_safe[toy.token("\"}") as usize]);
        assert!(!tables.text_safe[toy.token("\n") as usize]);
        assert!(tables.text_safe[toy.token("Notes") as usize]);
    }
}
