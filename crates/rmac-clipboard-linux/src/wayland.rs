//! The Wayland clipboard through wl-clipboard's `wl-paste` and `wl-copy`.
//!
//! `wl-paste --watch` binds ext-data-control-v1 or wlr-data-control-unstable
//! -v1 (niri 26.04 offers both) and runs a command for every new selection.
//! That command only drains the offer and prints wl-paste's
//! `CLIPBOARD_STATE` word, so no clipboard content passes through the
//! shell or this process's pipe. The service then asks for the offered
//! types and reads the single one it keeps.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::process::{Command, Stdio};

use crate::Error;

const WL_PASTE: &str = "wl-paste";
const WL_COPY: &str = "wl-copy";
/// Every read is bounded in time: a source client that never finishes
/// writing must not stall the history.
const TIMEOUT: &str = "timeout";
const READ_SECONDS: &str = "5";

/// Run by wl-paste for each selection. wl-clipboard 2.2 sets
/// `CLIPBOARD_STATE` to `data`, `nil`, `clear` or `sensitive` (the
/// password-manager hint); older versions leave it unset, meaning data.
pub const WATCH_SCRIPT: &str = "cat >/dev/null; printf '%s\\n' \"${CLIPBOARD_STATE:-data}\"";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    Data,
    Sensitive,
    Cleared,
}

pub fn parse_state(line: &str) -> Option<Change> {
    match line.trim() {
        "data" => Some(Change::Data),
        "sensitive" => Some(Change::Sensitive),
        "nil" | "clear" => Some(Change::Cleared),
        _ => None,
    }
}

/// Forward clipboard changes until the receiver closes (`Ok`) or wl-paste
/// stops (`Err`, so the caller can restart it). Blocking.
pub fn watch_blocking(sender: &async_channel::Sender<Change>) -> Result<(), Error> {
    let mut child = Command::new(WL_PASTE)
        .args(["--watch", "sh", "-c", WATCH_SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Clipboard)?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Clipboard);
    };
    let mut result = Err(Error::Clipboard);
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        if let Some(change) = parse_state(&line) {
            if sender.send_blocking(change).is_err() {
                result = Ok(());
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// The MIME types offered by the current selection; empty when there is
/// no selection.
pub fn offered_types() -> Result<Vec<String>, Error> {
    let output = Command::new(TIMEOUT)
        .args([READ_SECONDS, WL_PASTE, "--list-types"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| Error::Clipboard)?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Read the selection as exactly `mime`. `Ok(None)` when it is larger than
/// `limit` bytes (the read is abandoned, never truncated).
pub fn read(mime: &str, limit: u64) -> Result<Option<Vec<u8>>, Error> {
    let mut child = Command::new(TIMEOUT)
        .args([READ_SECONDS, WL_PASTE, "--no-newline", "--type", mime])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Clipboard)?;
    let mut bytes = Vec::new();
    let read = match child.stdout.take() {
        Some(stdout) => stdout.take(limit + 1).read_to_end(&mut bytes).is_ok(),
        None => false,
    };
    if !read || bytes.len() as u64 > limit {
        let _ = child.kill();
        let _ = child.wait();
        return if read {
            Ok(None)
        } else {
            Err(Error::Clipboard)
        };
    }
    let status = child.wait().map_err(|_| Error::Clipboard)?;
    if status.success() {
        Ok(Some(bytes))
    } else {
        Err(Error::Clipboard)
    }
}

/// Put `bytes` on the clipboard as `mime`. wl-copy reads everything, then
/// keeps serving the selection from a background process.
pub fn write(mime: &str, bytes: &[u8]) -> Result<(), Error> {
    let mut child = Command::new(WL_COPY)
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Clipboard)?;
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(bytes).is_ok())
        .unwrap_or(false);
    let status = child.wait().map_err(|_| Error::Clipboard)?;
    (written && status.success())
        .then_some(())
        .ok_or(Error::Clipboard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_states_map_to_changes() {
        assert_eq!(parse_state("data\n"), Some(Change::Data));
        assert_eq!(parse_state("sensitive"), Some(Change::Sensitive));
        assert_eq!(parse_state("nil"), Some(Change::Cleared));
        assert_eq!(parse_state("clear"), Some(Change::Cleared));
        assert_eq!(parse_state("something else"), None);
    }

    #[test]
    fn the_watch_script_never_echoes_clipboard_content() {
        assert!(WATCH_SCRIPT.starts_with("cat >/dev/null;"));
        assert!(!WATCH_SCRIPT.contains("$(cat"));
    }
}
