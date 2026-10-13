//! How a transcript reaches Spotlight.
//!
//! The talk-key HUD writes the transcript to a private file in the
//! session's runtime directory (tmpfs, mode 0600) and then opens Spotlight
//! through its ordinary shortcut. Spotlight takes the file as it opens,
//! deleting it, and shows the text as if it had been typed: the same
//! search, the same closed intents, the same "Lulo can do this" row and the
//! same second Return before a Settings change. A transcript older than
//! [`FRESH_FOR`] is ignored, so a stale file can never fill a later
//! Spotlight.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long a transcript waits for Spotlight to take it.
pub const FRESH_FOR: Duration = Duration::from_secs(10);

/// The longest transcript handed over, in bytes: a request, not a speech.
pub const MAX_TEXT_BYTES: usize = 400;

#[derive(Debug, Deserialize, Serialize)]
struct Handoff {
    text: String,
    written_unix_ms: u64,
}

/// `$XDG_RUNTIME_DIR/rmac/voice-transcript.json`.
pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|runtime| runtime.join("rmac/voice-transcript.json"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Cut `text` to [`MAX_TEXT_BYTES`] on a character boundary, trimmed.
fn bounded(text: &str) -> String {
    let text = text.trim();
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_owned()
}

/// Leave `text` for the next Spotlight.
pub fn write(text: &str) -> std::io::Result<()> {
    let path = path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
    })?;
    write_at(&path, text, now_ms())
}

pub fn write_at(path: &Path, text: &str, written_unix_ms: u64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec(&Handoff {
        text: bounded(text),
        written_unix_ms,
    })
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

/// Take a fresh transcript, if one is waiting. The file is removed either
/// way, so it is used at most once.
pub fn take() -> Option<String> {
    take_at(&path()?, now_ms())
}

pub fn take_at(path: &Path, now_unix_ms: u64) -> Option<String> {
    let bytes = rmac_storage::read_bounded_no_follow(path, 4096).ok()?;
    let _ = std::fs::remove_file(path);
    let handoff: Handoff = serde_json::from_slice(&bytes).ok()?;
    let age = now_unix_ms.checked_sub(handoff.written_unix_ms)?;
    if age > FRESH_FOR.as_millis() as u64 {
        return None;
    }
    let text = bounded(&handoff.text);
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_is_taken_once_and_only_while_fresh() {
        let directory =
            std::env::temp_dir().join(format!("rmac-voice-handoff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("rmac/voice-transcript.json");
        assert_eq!(take_at(&path, 1_000), None);
        write_at(&path, "  turn on dark mode \n", 1_000).unwrap();
        assert_eq!(take_at(&path, 1_500), Some("turn on dark mode".into()));
        assert!(!path.exists(), "taken once");
        assert_eq!(take_at(&path, 1_500), None);
        // Too old, from the future, or empty: ignored, and still removed.
        write_at(&path, "open notes", 1_000).unwrap();
        assert_eq!(take_at(&path, 1_000 + 10_001), None);
        assert!(!path.exists());
        write_at(&path, "open notes", 5_000).unwrap();
        assert_eq!(take_at(&path, 1_000), None);
        write_at(&path, "   ", 1_000).unwrap();
        assert_eq!(take_at(&path, 1_000), None);
        // Long text is cut on a character boundary.
        write_at(&path, &"é".repeat(300), 1_000).unwrap();
        let taken = take_at(&path, 1_000).unwrap();
        assert!(taken.len() <= MAX_TEXT_BYTES);
        assert!(taken.chars().all(|character| character == 'é'));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            write_at(&path, "x", 1).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
