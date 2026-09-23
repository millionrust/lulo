//! Archive Utility for rmac: expand archives next to themselves and compress
//! Finder selections into zip files, with the Mac's names and messages.
//!
//! Measured on macOS 26.2 (design-lab/archive.html):
//! - An archive with several top-level items expands into a folder named
//!   after it ("Bundle.zip" → "Bundle", then "Bundle 2"); an archive holding
//!   one item expands to that item ("notes.txt", or "notes 2.txt" when the
//!   name is taken).
//! - Compressing one item makes "name.zip"; several make "Archive.zip", then
//!   "Archive 2.zip".
//! - A file that is not an archive fails with "Unable to expand “X”. It is
//!   in an unsupported format."
//!
//! Why pure Rust (zip, tar, flate2, bzip2, lzma-rs) rather than `bsdtar`:
//! Ubuntu installs neither libarchive-tools nor zip/unzip by default, so a
//! subprocess would fail on a fresh system, and a library keeps progress,
//! cancellation and path checks inside rmac. Every entry is written below a
//! private staging folder next to the archive, symbolic links are created
//! only after every regular entry, and nothing appears under its final name
//! until the whole archive has expanded, so a failure or cancel leaves the
//! folder exactly as it was.

mod compress;
mod expand;
mod naming;
mod staging;

use std::fmt;
use std::io;
use std::path::Path;

pub use compress::{compress, compressed_name};
pub use expand::expand;
pub use naming::{archive_stem, format_of, unique_path, Format};

/// Bytes processed so far out of the job's total (both in the units the job
/// can measure up front: compressed bytes read when expanding, source bytes
/// read when compressing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug)]
pub enum Error {
    /// The file is not an archive rmac can read.
    Unsupported,
    /// The archive is damaged part-way through.
    Damaged,
    /// The user stopped the job.
    Cancelled,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("unsupported format"),
            Self::Damaged => formatter.write_str("damaged archive"),
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::Interrupted && error.to_string() == CANCELLED {
            Self::Cancelled
        } else {
            Self::Io(error)
        }
    }
}

const CANCELLED: &str = "rmac-archive: cancelled";

pub(crate) fn cancelled_io() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, CANCELLED)
}

/// The Archive Utility alert text for a failed expansion, or `None` when the
/// user cancelled (the Mac shows nothing then).
pub fn expand_error_message(archive: &Path, error: &Error) -> Option<String> {
    let name = display_name(archive);
    let folder = archive
        .parent()
        .map(display_name)
        .unwrap_or_else(|| "/".to_owned());
    match error {
        Error::Cancelled => None,
        Error::Unsupported => Some(format!(
            "Unable to expand “{name}”. It is in an unsupported format."
        )),
        Error::Damaged => Some(format!(
            "Unable to expand “{name}” into “{folder}”. (Error 79 - Inappropriate file type or format.)"
        )),
        Error::Io(error) => Some(format!(
            "Unable to expand “{name}” into “{folder}”. {}",
            errno_suffix(error)
        )),
    }
}

/// The alert text for a failed compression, or `None` when cancelled.
pub fn compress_error_message(items: &[std::path::PathBuf], error: &Error) -> Option<String> {
    let subject = match items {
        [one] => format!("“{}”", display_name(one)),
        _ => format!("{} items", items.len()),
    };
    let folder = items
        .first()
        .and_then(|item| item.parent())
        .map(display_name)
        .unwrap_or_else(|| "/".to_owned());
    match error {
        Error::Cancelled => None,
        Error::Unsupported | Error::Damaged => Some(format!(
            "Unable to archive {subject} into “{folder}”. (Error 79 - Inappropriate file type or format.)"
        )),
        Error::Io(error) => Some(format!(
            "Unable to archive {subject} into “{folder}”. {}",
            errno_suffix(error)
        )),
    }
}

/// "(Error 13 - Permission denied.)" — the Mac's shape with the system's
/// own errno text.
fn errno_suffix(error: &io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => {
            let text = io::Error::from_raw_os_error(code).to_string();
            let reason = text
                .split(" (os error")
                .next()
                .unwrap_or(&text)
                .trim()
                .to_owned();
            format!("(Error {code} - {reason}.)")
        }
        None => format!("({}.)", sentence(&error.to_string())),
    }
}

fn sentence(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches('.');
    let mut characters = trimmed.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Unknown error".to_owned(),
    }
}

/// Finder's decimal sizes: "1 byte", "364 bytes", "10 KB", "120.0 MB",
/// "1.2 GB" (the Mac's Quick Look and copy window use these).
pub fn size_label(bytes: u64) -> String {
    let value = bytes as f64;
    if bytes == 1 {
        "1 byte".to_owned()
    } else if bytes < 1_000 {
        format!("{bytes} bytes")
    } else if bytes < 1_000_000 {
        format!("{} KB", (value / 1e3).round() as u64)
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", value / 1e6)
    } else if bytes < 1_000_000_000_000 {
        format!("{:.1} GB", value / 1e9)
    } else {
        format!("{:.1} TB", value / 1e12)
    }
}

/// The progress line under the bar: "120.0 MB of 1.2 GB – Less than a
/// minute", later "630.2 MB of 1.2 GB – About 10 seconds" (both measured on
/// the Mac's copy window). S: the Mac's estimate rounding is private; this
/// says "Less than a minute" for the first two seconds and then rounds up to
/// 5 s.
pub fn progress_line(progress: Progress, elapsed: std::time::Duration) -> String {
    let done = size_label(progress.done);
    let total = size_label(progress.total);
    let seconds = elapsed.as_secs_f64();
    let estimate = if progress.done == 0 || seconds < 2.0 {
        "Less than a minute".to_owned()
    } else {
        let rate = progress.done as f64 / seconds;
        let remaining = progress.total.saturating_sub(progress.done) as f64 / rate.max(1.0);
        if remaining >= 90.0 {
            format!("About {} minutes", (remaining / 60.0).round() as u64)
        } else if remaining >= 55.0 {
            "About a minute".to_owned()
        } else {
            format!(
                "About {} seconds",
                ((remaining / 5.0).ceil() as u64 * 5).max(5)
            )
        }
    };
    format!("{done} of {total} – {estimate}")
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn alert_wording_matches_the_mac() {
        assert_eq!(
            expand_error_message(Path::new("/home/me/qlab/Broken.zip"), &Error::Unsupported)
                .as_deref(),
            Some("Unable to expand “Broken.zip”. It is in an unsupported format.")
        );
        assert_eq!(
            expand_error_message(
                Path::new("/home/me/qlab/Bundle.zip"),
                &Error::Io(io::Error::from_raw_os_error(libc::EACCES))
            )
            .as_deref(),
            Some(
                format!(
                    "Unable to expand “Bundle.zip” into “qlab”. (Error {} - Permission denied.)",
                    libc::EACCES
                )
                .as_str()
            )
        );
        assert!(expand_error_message(Path::new("/a/b.zip"), &Error::Cancelled).is_none());
        assert_eq!(
            compress_error_message(
                &[PathBuf::from("/x/a"), PathBuf::from("/x/b")],
                &Error::Io(io::Error::from_raw_os_error(libc::ENOSPC))
            )
            .as_deref()
            .map(|text| text.starts_with("Unable to archive 2 items into “x”. (Error ")),
            Some(true)
        );
    }

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(size_label(1), "1 byte");
        assert_eq!(size_label(364), "364 bytes");
        assert_eq!(size_label(10_240), "10 KB");
        assert_eq!(size_label(120_000_000), "120.0 MB");
        assert_eq!(size_label(630_200_000), "630.2 MB");
        assert_eq!(size_label(1_200_000_000), "1.2 GB");
    }

    #[test]
    fn progress_reads_like_the_macs_copy_window() {
        let start = Progress {
            done: 120_000_000,
            total: 1_200_000_000,
        };
        assert_eq!(
            progress_line(start, std::time::Duration::from_millis(500)),
            "120.0 MB of 1.2 GB – Less than a minute"
        );
        let halfway = Progress {
            done: 630_200_000,
            total: 1_200_000_000,
        };
        assert_eq!(
            progress_line(halfway, std::time::Duration::from_secs(10)),
            "630.2 MB of 1.2 GB – About 10 seconds"
        );
    }

    #[test]
    fn cancellation_survives_io_conversion() {
        assert!(matches!(Error::from(cancelled_io()), Error::Cancelled));
        assert!(matches!(
            Error::from(io::Error::new(io::ErrorKind::Interrupted, "signal")),
            Error::Io(_)
        ));
    }
}
