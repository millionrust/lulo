//! The bounded, newest-first clipboard history.

use serde::{Deserialize, Serialize};

use crate::{Draft, Entry, MAX_ENTRIES, MAX_TOTAL_BYTES, RETENTION_MS};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct History {
    /// Newest first.
    entries: Vec<Entry>,
    next_id: u64,
}

/// What recording a copy changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recorded {
    pub id: u64,
    /// False when the copy matched an existing entry, which moved to the top
    /// and whose stored payload is reused.
    pub new_payload: bool,
    /// Entries dropped to stay within the bounds; their payloads must go.
    pub evicted: Vec<u64>,
}

impl History {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Record a copy at `now_ms`. Copying something already in the history
    /// (including putting an old entry back) moves it to the top.
    pub fn record(&mut self, draft: Draft, now_ms: u64) -> Recorded {
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.digest == draft.digest && entry.kind == draft.kind && entry.size == draft.size
        }) {
            let mut entry = self.entries.remove(index);
            entry.copied_at_ms = now_ms;
            let id = entry.id;
            self.entries.insert(0, entry);
            return Recorded {
                id,
                new_payload: false,
                evicted: Vec::new(),
            };
        }
        self.next_id = self.next_id.max(self.max_id()).wrapping_add(1).max(1);
        let id = self.next_id;
        self.entries.insert(
            0,
            Entry {
                id,
                kind: draft.kind,
                mime: draft.mime,
                title: draft.title,
                detail: draft.detail,
                size: draft.size,
                digest: draft.digest,
                copied_at_ms: now_ms,
            },
        );
        let evicted = self.enforce_bounds();
        Recorded {
            id,
            new_payload: true,
            evicted,
        }
    }

    /// Forget entries older than the retention window, and any whose clock
    /// lies in the future (the wall clock moved backwards).
    pub fn expire(&mut self, now_ms: u64) -> Vec<u64> {
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            let keep = entry.copied_at_ms <= now_ms
                && now_ms.saturating_sub(entry.copied_at_ms) < RETENTION_MS;
            if !keep {
                removed.push(entry.id);
            }
            keep
        });
        removed
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Drop entries whose payload is gone (for example after a crash).
    pub fn retain_payloads(&mut self, mut present: impl FnMut(u64) -> bool) {
        self.entries.retain(|entry| present(entry.id));
    }

    pub fn clear(&mut self) -> Vec<u64> {
        self.entries.drain(..).map(|entry| entry.id).collect()
    }

    /// Repair a history read from disk: drop repeated and zero ids (a repeat
    /// shares its payload with the kept entry), then re-apply the bounds.
    /// Returns the ids whose payloads must go.
    pub fn normalise(&mut self) -> Vec<u64> {
        let mut seen = std::collections::BTreeSet::new();
        self.entries
            .retain(|entry| entry.id != 0 && seen.insert(entry.id));
        self.next_id = self.next_id.max(self.max_id());
        self.enforce_bounds()
    }

    fn max_id(&self) -> u64 {
        self.entries.iter().map(|entry| entry.id).max().unwrap_or(0)
    }

    fn enforce_bounds(&mut self) -> Vec<u64> {
        let mut evicted = Vec::new();
        let mut total: u64 = self.entries.iter().map(|entry| entry.size).sum();
        while self.entries.len() > MAX_ENTRIES
            || (total > MAX_TOTAL_BYTES && self.entries.len() > 1)
        {
            let Some(entry) = self.entries.pop() else {
                break;
            };
            total = total.saturating_sub(entry.size);
            evicted.push(entry.id);
        }
        evicted
    }
}
