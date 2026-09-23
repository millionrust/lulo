//! Clipboard history entries, limits, and their stable D-Bus encoding.

use serde::{Deserialize, Serialize};

/// Most entries kept. Older entries are evicted first.
pub const MAX_ENTRIES: usize = 100;
/// Total payload bytes kept across all entries.
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Largest single payload recorded; larger copies are skipped, not truncated.
pub const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
/// Largest text payload recorded.
pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// Entries older than this are forgotten, even within the session.
pub const RETENTION_MS: u64 = 8 * 60 * 60 * 1000;
/// Characters of a text entry's first line shown as its title.
pub const PREVIEW_CHARS: usize = 200;
/// KDE's password-manager hint. KeePassXC, Bitwarden and other password
/// managers offer it with the value `secret` beside the copied password.
pub const SENSITIVE_HINT_MIME: &str = "x-kde-passwordManagerHint";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Text,
    Image,
    Files,
}

impl Kind {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "files" => Some(Self::Files),
            _ => None,
        }
    }

    /// The word shown in a row's subtitle.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Files => "File",
        }
    }
}

/// A recorded clipboard item. The payload itself lives beside the history,
/// keyed by `id`; this record carries only what the list displays.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Entry {
    pub id: u64,
    pub kind: Kind,
    /// The exact MIME type the payload was read as, used to put it back.
    pub mime: String,
    /// Title: a text item's first line, a file's name, or the image type.
    pub title: String,
    /// Extra detail: image dimensions or the files' folder. May be empty.
    pub detail: String,
    pub size: u64,
    pub digest: u64,
    pub copied_at_ms: u64,
}

/// A classified payload that has not been given an identity yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft {
    pub kind: Kind,
    pub mime: String,
    pub title: String,
    pub detail: String,
    pub size: u64,
    pub digest: u64,
}

/// id, kind, MIME type, title, detail, size, copied-at (Unix ms), payload path.
pub type WireEntry = (u64, String, String, String, String, u64, u64, String);

/// An entry as a client receives it, with the private payload's path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub entry: Entry,
    pub payload: std::path::PathBuf,
}

pub fn encode(entry: &Entry, payload: &std::path::Path) -> WireEntry {
    (
        entry.id,
        entry.kind.wire().to_owned(),
        entry.mime.clone(),
        entry.title.clone(),
        entry.detail.clone(),
        entry.size,
        entry.copied_at_ms,
        payload.to_string_lossy().into_owned(),
    )
}

pub fn decode(wire: WireEntry) -> Option<Item> {
    let (id, kind, mime, title, detail, size, copied_at_ms, payload) = wire;
    let payload = std::path::PathBuf::from(payload);
    (id != 0 && payload.is_absolute()).then_some(())?;
    Some(Item {
        entry: Entry {
            id,
            kind: Kind::from_wire(&kind)?,
            mime,
            title,
            detail,
            size,
            digest: 0,
            copied_at_ms,
        },
        payload,
    })
}

/// "Just now", "1 min ago", "3 hr ago": the age shown in a row's subtitle.
pub fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(then_ms) / 60_000;
    match minutes {
        0 => "Just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        _ => format!("{} hr ago", minutes / 60),
    }
}

/// The subtitle of a clipboard row: kind, optional detail, and age.
pub fn subtitle(entry: &Entry, now_ms: u64) -> String {
    let age = relative_age(now_ms, entry.copied_at_ms);
    if entry.detail.is_empty() {
        format!("{} · {age}", entry.kind.label())
    } else {
        format!("{} · {} · {age}", entry.kind.label(), entry.detail)
    }
}
