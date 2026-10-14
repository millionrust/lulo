//! Screen On Usage, the Mac Battery pane's second chart: how many minutes
//! of each hour (or day) the screen was on.
//!
//! On Lulo OS the session's lock coordinator records each time the screen
//! comes on (session start, wake from sleep) and goes off (sleep,
//! shutdown) as one line of `<unix seconds> on|off` in
//! `$XDG_STATE_HOME/rmac/screen-on.log`. Nothing is sampled; only those
//! events are written. A session that ended without an "off" (power loss,
//! a crash) is closed at the last battery sample UPower recorded before the
//! next "on", or counted as nothing when there is none.

use std::io::Write as _;
use std::path::PathBuf;

/// Lines older than this are dropped when the log is rewritten.
const KEEP_SECONDS: u64 = 11 * 24 * 60 * 60;
/// The log is rewritten (pruned) once it grows past this.
const PRUNE_BYTES: u64 = 64 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;
/// How long after its last battery sample an unclosed session may have run.
const SAMPLE_GRACE_SECONDS: u64 = 10 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenEvent {
    pub at: u64,
    pub on: bool,
}

fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })?;
    Some(base.join("rmac").join("screen-on.log"))
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Record that the screen came on (`true`) or went off (`false`) now.
pub fn record(on: bool) -> std::io::Result<()> {
    let path = log_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no state directory"))?;
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let now = now_seconds();
    let line = format!("{now} {}\n", if on { "on" } else { "off" });
    let large = std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > PRUNE_BYTES);
    if large {
        let mut kept = parse(&read_log(&path))
            .into_iter()
            .filter(|event| event.at + KEEP_SECONDS >= now)
            .map(|event| format!("{} {}\n", event.at, if event.on { "on" } else { "off" }))
            .collect::<String>();
        kept.push_str(&line);
        return rmac_storage::atomic_write_private(&path, kept.as_bytes());
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())
}

fn read_log(path: &std::path::Path) -> String {
    rmac_storage::read_bounded_no_follow(path, MAX_READ_BYTES)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

pub(crate) fn parse(text: &str) -> Vec<ScreenEvent> {
    let mut events = text
        .lines()
        .filter_map(|line| {
            let (at, state) = line.split_once(' ')?;
            let on = match state.trim() {
                "on" => true,
                "off" => false,
                _ => return None,
            };
            Some(ScreenEvent {
                at: at.parse().ok()?,
                on,
            })
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.at);
    events
}

/// The recorded events, oldest first. Empty when nothing was recorded
/// (Windows, macOS, or a session without the lock coordinator).
pub fn events() -> Vec<ScreenEvent> {
    log_path()
        .map(|path| parse(&read_log(&path)))
        .unwrap_or_default()
}

/// Screen-on intervals from the events. `samples` are timestamps known to
/// fall inside a running session (UPower's battery history).
pub fn intervals(events: &[ScreenEvent], samples: &[u64], now: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    for (index, event) in events.iter().enumerate() {
        if !event.on {
            continue;
        }
        let end = match events.get(index + 1) {
            Some(next) if !next.on => next.at,
            Some(next) => samples
                .iter()
                .copied()
                .filter(|sample| *sample > event.at && *sample < next.at)
                .max()
                .map_or(event.at, |last| (last + SAMPLE_GRACE_SECONDS).min(next.at)),
            None => now,
        };
        if end > event.at {
            out.push((event.at, end));
        }
    }
    out
}

/// Minutes of screen-on time in each of `buckets` buckets of
/// `bucket_seconds`, the first starting at `start`.
pub fn minutes_per_bucket(
    intervals: &[(u64, u64)],
    start: u64,
    bucket_seconds: u64,
    buckets: usize,
) -> Vec<u32> {
    (0..buckets as u64)
        .map(|bucket| {
            let from = start + bucket * bucket_seconds;
            let to = from + bucket_seconds;
            let seconds: u64 = intervals
                .iter()
                .map(|(a, b)| (*b).min(to).saturating_sub((*a).max(from)))
                .sum();
            (seconds / 60) as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_and_open_sessions_are_counted() {
        let events = parse("100 on\n700 off\n1000 on\n");
        assert_eq!(
            intervals(&events, &[], 1300),
            vec![(100, 700), (1000, 1300)]
        );
    }

    #[test]
    fn a_session_without_an_off_ends_at_its_last_battery_sample() {
        let events = parse("0 on\n100000 on\n");
        let ends = intervals(&events, &[600, 1200], 100100);
        assert_eq!(
            ends,
            vec![(0, 1200 + SAMPLE_GRACE_SECONDS), (100000, 100100)]
        );
        // No sample: nothing is claimed for the unclosed session.
        assert_eq!(intervals(&events, &[], 100100), vec![(100000, 100100)]);
    }

    #[test]
    fn minutes_split_across_hours() {
        let minutes = minutes_per_bucket(&[(1800, 5400)], 0, 3600, 3);
        assert_eq!(minutes, vec![30, 30, 0]);
    }

    #[test]
    fn junk_lines_are_ignored() {
        assert_eq!(
            parse("x on\n5 maybe\n7 off\n"),
            vec![ScreenEvent { at: 7, on: false }]
        );
    }
}
