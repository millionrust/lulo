//! Local, private learning from the results a person chooses.
//!
//! macOS moves a result up once it has been chosen for a query ("te" →
//! Terminal after Terminal was opened from "term"). rmac keeps the same
//! memory on this computer only: the normalised query, the chosen result's
//! identity, how often and when it was last chosen. Nothing is sent
//! anywhere, entries fade with a two-week half-life and are forgotten after
//! ninety days, and the store never holds more than [`MAX_CHOICES`] entries.

use crate::engine::normalize;
use crate::ResultId;

/// Most choices remembered; the least recently used go first.
pub const MAX_CHOICES: usize = 256;
/// Largest score a learned choice can add. An exact title match scores
/// 1000 and a prefix match 850, so a well-used prefix choice can overtake
/// an exact match the person never picks, but not by a wide margin.
pub const MAX_BOOST: u16 = 300;
const MAX_QUERY_CHARS: usize = 64;
const HALF_LIFE_SECONDS: f64 = 14.0 * 86_400.0;
const FORGET_AFTER_SECONDS: u64 = 90 * 86_400;
/// Choices at which the frequency factor saturates.
const FREQUENT: u32 = 8;
const HEADER: &str = "rmac-spotlight-choices 1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Choice {
    /// Normalised query (lower case, single spaces), at most 64 characters.
    pub query: String,
    pub id: ResultId,
    pub count: u32,
    /// Unix seconds.
    pub last_used: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Learning {
    choices: Vec<Choice>,
}

impl Learning {
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    pub fn is_empty(&self) -> bool {
        self.choices.is_empty()
    }

    /// Remember that `id` was chosen for `query` at `now`.
    pub fn record(&mut self, query: &str, id: &ResultId, now: u64) {
        let query = learned_query(query);
        if query.is_empty() || id.local.trim().is_empty() {
            return;
        }
        self.prune(now);
        if let Some(choice) = self
            .choices
            .iter_mut()
            .find(|choice| choice.query == query && &choice.id == id)
        {
            choice.count = choice.count.saturating_add(1);
            choice.last_used = choice.last_used.max(now);
        } else {
            self.choices.push(Choice {
                query,
                id: id.clone(),
                count: 1,
                last_used: now,
            });
        }
        self.enforce_capacity();
    }

    /// The learned score for `id` under `query`: 0 when it was never chosen
    /// for this query or one it extends or shortens.
    pub fn boost(&self, query: &str, id: &ResultId, now: u64) -> u16 {
        let query = learned_query(query);
        if query.is_empty() {
            return 0;
        }
        let total: f64 = self
            .choices
            .iter()
            .filter(|choice| &choice.id == id)
            .map(|choice| {
                let relation = if choice.query == query {
                    1.0
                } else if choice.query.starts_with(&query) {
                    // Typed less than last time ("te" after "term").
                    0.8
                } else if query.starts_with(&choice.query) {
                    // Typed more than last time ("terminal" after "term").
                    0.6
                } else {
                    return 0.0;
                };
                let frequency = (1.0 + f64::from(choice.count.min(FREQUENT)).ln())
                    / (1.0 + f64::from(FREQUENT).ln());
                let age = now.saturating_sub(choice.last_used) as f64;
                let decay = 0.5_f64.powf(age / HALF_LIFE_SECONDS);
                relation * frequency * decay
            })
            .sum();
        (f64::from(MAX_BOOST) * total.min(1.0)).round() as u16
    }

    /// Drop every choice of `id` (for example a file that no longer exists).
    pub fn forget(&mut self, id: &ResultId) {
        self.choices.retain(|choice| &choice.id != id);
    }

    /// Drop choices older than ninety days.
    pub fn prune(&mut self, now: u64) {
        self.choices
            .retain(|choice| now.saturating_sub(choice.last_used) <= FORGET_AFTER_SECONDS);
    }

    fn enforce_capacity(&mut self) {
        if self.choices.len() <= MAX_CHOICES {
            return;
        }
        self.choices
            .sort_by_key(|choice| std::cmp::Reverse(choice.last_used));
        self.choices.truncate(MAX_CHOICES);
    }

    /// One line per choice: count, last use, provider, result, query;
    /// tab-separated with `\t`, `\n` and `\\` escaped.
    pub fn to_text(&self) -> String {
        let mut text = String::from(HEADER);
        text.push('\n');
        for choice in &self.choices {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                choice.count,
                choice.last_used,
                escape(&choice.id.provider.0),
                escape(&choice.id.local),
                escape(&choice.query),
            ));
        }
        text
    }

    /// Read [`to_text`](Self::to_text). Malformed lines are skipped and an
    /// unknown header yields an empty store, so a damaged file only costs
    /// what was learned.
    pub fn from_text(text: &str) -> Self {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Self::default();
        }
        let mut learning = Self::default();
        for line in lines {
            let fields = line.split('\t').collect::<Vec<_>>();
            let [count, last_used, provider, local, query] = fields.as_slice() else {
                continue;
            };
            let (Ok(count), Ok(last_used), Some(provider), Some(local), Some(query)) = (
                count.parse::<u32>(),
                last_used.parse::<u64>(),
                unescape(provider),
                unescape(local),
                unescape(query),
            ) else {
                continue;
            };
            let query = learned_query(&query);
            if count == 0 || provider.is_empty() || local.trim().is_empty() || query.is_empty() {
                continue;
            }
            learning.choices.push(Choice {
                query,
                id: ResultId {
                    provider: rmac_shell_settings::ProviderId(provider),
                    local,
                },
                count,
                last_used,
            });
        }
        learning.enforce_capacity();
        learning
    }
}

fn learned_query(query: &str) -> String {
    normalize(query).chars().take(MAX_QUERY_CHARS).collect()
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn unescape(value: &str) -> Option<String> {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        unescaped.push(match characters.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(unescaped)
}
