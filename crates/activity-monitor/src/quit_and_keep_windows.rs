//! Application ▸ Quit and Keep Windows (MON-16, MON-MENU-001): like
//! Preview's and Notes' own session restore, persists just which floating
//! metric windows (`floating_windows.rs`) were open, then reopens exactly
//! those once on the next launch. A plain Quit never writes this file, so
//! a plain next launch starts with no floating windows, matching every
//! other app's "Quit and Keep Windows" being the one that opts in to
//! restoring anything.

use std::io;
use std::path::{Path, PathBuf};

use crate::floating_windows::MetricWindowKind;

fn state_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let root = match std::env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".local/state"),
    };
    Some(root.join("rmac-system-monitor/kept-windows.txt"))
}

/// One id per line, the same simple format `columns.rs` already uses for
/// its own preferences file (plain text, no new serialization dependency).
fn format(kinds: &[MetricWindowKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.storage_id())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse(content: &str) -> Vec<MetricWindowKind> {
    content
        .lines()
        .filter_map(|line| MetricWindowKind::from_storage_id(line.trim()))
        .collect()
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, path)
}

/// Persist which floating windows are open right now. Called only from
/// the Quit and Keep Windows handler, never from an ordinary quit/close.
pub(crate) fn save(open: &[MetricWindowKind]) {
    let Some(path) = state_path() else {
        return;
    };
    if let Err(error) = write_atomic(&path, &format(open)) {
        eprintln!("rmac-system-monitor: could not keep windows: {error}");
    }
}

/// Consume the saved set once — read it and delete the file in the same
/// step, so a later ordinary quit can't accidentally "restore" a stale
/// list on the launch after next.
pub(crate) fn take_saved() -> Vec<MetricWindowKind> {
    let Some(path) = state_path() else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    parse(&contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_round_trip_every_kind() {
        let all = MetricWindowKind::ALL.to_vec();
        assert_eq!(parse(&format(&all)), all);
    }

    #[test]
    fn parse_ignores_blank_and_unknown_lines() {
        assert_eq!(
            parse("cpu-usage\n\nnot-a-real-kind\ngpu-history\n"),
            vec![MetricWindowKind::CpuUsage, MetricWindowKind::GpuHistory]
        );
    }

    #[test]
    fn empty_saved_state_restores_nothing() {
        assert_eq!(parse(""), Vec::new());
    }
}
