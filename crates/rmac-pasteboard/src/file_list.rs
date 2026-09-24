//! File lists in the clipboard formats Linux file managers exchange.
//!
//! * `text/uri-list` (RFC 2483): one `file://` URI per line, CRLF-separated,
//!   `#` lines are comments. Nautilus, Dolphin, Thunar and browsers read it.
//! * `x-special/gnome-copied-files`: `copy` or `cut`, then one URI per line.
//!   Nautilus and Thunar use it to tell a cut from a copy.
//! * `application/x-kde-cutselection`: `1` next to a `text/uri-list` when
//!   Dolphin cut the files.
//!
//! URIs follow RFC 8089: `file://` with an empty (or `localhost`) host and
//! a percent-encoded absolute path. Unix paths are bytes, so every byte
//! outside the unreserved set is encoded and decoding yields raw bytes, not
//! UTF-8.

// Only Linux reads these formats; macOS uses NSPasteboard's file URLs.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

pub const URI_LIST: &str = "text/uri-list";
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

/// Files on the clipboard, and whether they were cut (paste moves them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileList {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// Why clipboard data could not be read as local files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The data is not UTF-8 (URIs are ASCII once encoded).
    NotText,
    /// An entry is not a `file://` URI on this computer.
    NotLocalFile(String),
    /// A `%` escape is incomplete or not hexadecimal, or decodes to NUL.
    BadEscape(String),
    /// A GNOME list does not start with `copy` or `cut`.
    UnknownAction(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotText => write!(f, "the clipboard's file list is not text"),
            Self::NotLocalFile(uri) => {
                write!(f, "“{}” is not a file on this computer", shorten(uri))
            }
            Self::BadEscape(uri) => write!(f, "“{}” is not a valid file address", shorten(uri)),
            Self::UnknownAction(action) => {
                write!(
                    f,
                    "the clipboard asks for “{}”, not copy or cut",
                    shorten(action)
                )
            }
        }
    }
}

fn shorten(value: &str) -> String {
    const LIMIT: usize = 80;
    if value.chars().count() <= LIMIT {
        value.to_owned()
    } else {
        let mut short: String = value.chars().take(LIMIT).collect();
        short.push('…');
        short
    }
}

/// `file://` URI for an absolute path; `None` for a relative one.
pub fn file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = path.as_os_str().as_bytes();
    let mut uri = String::with_capacity("file://".len() + bytes.len());
    uri.push_str("file://");
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push(char::from(HEX[usize::from(byte >> 4)]));
            uri.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    Some(uri)
}

/// The path a local `file:` URI names. Accepts `file:///p`,
/// `file://localhost/p` and the RFC 8089 short form `file:/p`.
pub fn file_uri_path(uri: &str) -> Result<PathBuf, ParseError> {
    let not_local = || ParseError::NotLocalFile(uri.to_owned());
    let scheme_end = uri.find(':').ok_or_else(not_local)?;
    if !uri[..scheme_end].eq_ignore_ascii_case("file") {
        return Err(not_local());
    }
    let rest = &uri[scheme_end + 1..];
    let encoded = if let Some(authority_and_path) = rest.strip_prefix("//") {
        let slash = authority_and_path.find('/').ok_or_else(not_local)?;
        let host = &authority_and_path[..slash];
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return Err(not_local());
        }
        &authority_and_path[slash..]
    } else if rest.starts_with('/') {
        rest
    } else {
        return Err(not_local());
    };
    // A query or fragment has no meaning for a local file.
    if encoded.contains(['?', '#']) {
        return Err(not_local());
    }
    let bytes = percent_decode(encoded).ok_or_else(|| ParseError::BadEscape(uri.to_owned()))?;
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

fn percent_decode(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            index += 3;
            high * 16 + low
        } else {
            index += 1;
            bytes[index - 1]
        };
        if byte == 0 {
            return None;
        }
        decoded.push(byte);
    }
    Some(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn uris(paths: &[PathBuf]) -> impl Iterator<Item = String> + '_ {
    paths.iter().filter_map(|path| file_uri(path))
}

/// `text/uri-list` payload: CRLF after every URI, as RFC 2483 asks.
pub fn format_uri_list(paths: &[PathBuf]) -> String {
    uris(paths).map(|uri| uri + "\r\n").collect()
}

/// `x-special/gnome-copied-files` payload, byte-for-byte what Nautilus
/// writes: the action, then `\n` and a URI for each file, no final newline.
pub fn format_gnome_copied_files(paths: &[PathBuf], cut: bool) -> String {
    let mut text = String::from(if cut { "cut" } else { "copy" });
    for uri in uris(paths) {
        text.push('\n');
        text.push_str(&uri);
    }
    text
}

fn entries(bytes: &[u8]) -> Result<impl Iterator<Item = &str>, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::NotText)?;
    Ok(text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty()))
}

/// Paths in a `text/uri-list`. Every entry must be a local file: a list
/// that mixes in web addresses is refused rather than half-pasted.
pub fn parse_uri_list(bytes: &[u8]) -> Result<Vec<PathBuf>, ParseError> {
    entries(bytes)?
        .filter(|line| !line.starts_with('#'))
        .map(file_uri_path)
        .collect()
}

/// A `x-special/gnome-copied-files` payload.
pub fn parse_gnome_copied_files(bytes: &[u8]) -> Result<FileList, ParseError> {
    let mut lines = entries(bytes)?;
    let cut = match lines.next() {
        Some("cut") => true,
        Some("copy") | None => false,
        Some(other) => return Err(ParseError::UnknownAction(other.to_owned())),
    };
    let paths = lines.map(file_uri_path).collect::<Result<_, _>>()?;
    Ok(FileList { paths, cut })
}

/// Dolphin's `application/x-kde-cutselection` marker: `1` means cut.
pub fn parse_kde_cut_selection(bytes: &[u8]) -> bool {
    bytes.trim_ascii() == b"1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(bytes: &[u8]) -> PathBuf {
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }

    #[test]
    fn uris_encode_everything_outside_the_unreserved_set() {
        assert_eq!(
            file_uri(Path::new("/home/me/My Report #2 (final).pdf")).as_deref(),
            Some("file:///home/me/My%20Report%20%232%20%28final%29.pdf")
        );
        assert_eq!(
            file_uri(Path::new("/tmp/a%b?c")).as_deref(),
            Some("file:///tmp/a%25b%3Fc")
        );
        assert_eq!(
            file_uri(Path::new("/tmp/café")).as_deref(),
            Some("file:///tmp/caf%C3%A9")
        );
        assert_eq!(
            file_uri(&path(b"/tmp/\xff\n")).as_deref(),
            Some("file:///tmp/%FF%0A")
        );
        assert_eq!(file_uri(Path::new("relative/name")), None);
    }

    #[test]
    fn uris_round_trip_arbitrary_path_bytes() {
        for bytes in [
            &b"/"[..],
            b"/home/me/Documents",
            b"/tmp/sp ace/%25/#hash?q",
            b"/tmp/\x01\x7f\x80\xfe\xff",
            "/tmp/日本語/ファイル.txt".as_bytes(),
        ] {
            let original = path(bytes);
            let uri = file_uri(&original).unwrap();
            assert!(uri.is_ascii(), "{uri}");
            assert_eq!(file_uri_path(&uri).unwrap(), original, "{uri}");
        }
    }

    #[test]
    fn only_local_file_uris_are_accepted() {
        assert_eq!(
            file_uri_path("file://localhost/etc/hosts").unwrap(),
            PathBuf::from("/etc/hosts")
        );
        assert_eq!(
            file_uri_path("FILE:///etc/hosts").unwrap(),
            PathBuf::from("/etc/hosts")
        );
        assert_eq!(
            file_uri_path("file:/etc/hosts").unwrap(),
            PathBuf::from("/etc/hosts")
        );
        for uri in [
            "https://example.com/a",
            "file://server/share/a",
            "file://",
            "file:relative",
            "sftp:///etc/hosts",
            "/etc/hosts",
            "file:///tmp/a?query",
            "file:///tmp/a#fragment",
        ] {
            assert!(
                matches!(file_uri_path(uri), Err(ParseError::NotLocalFile(_))),
                "{uri}"
            );
        }
        for uri in [
            "file:///tmp/%",
            "file:///tmp/%4",
            "file:///tmp/%zz",
            "file:///tmp/%00",
        ] {
            assert!(
                matches!(file_uri_path(uri), Err(ParseError::BadEscape(_))),
                "{uri}"
            );
        }
    }

    #[test]
    fn uri_lists_are_crlf_terminated_and_parse_back() {
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/c")];
        let list = format_uri_list(&paths);
        assert_eq!(list, "file:///tmp/a%20b\r\nfile:///tmp/c\r\n");
        assert_eq!(parse_uri_list(list.as_bytes()).unwrap(), paths);
    }

    #[test]
    fn uri_lists_from_other_apps_are_read_leniently_but_never_half_pasted() {
        // Bare LF, a comment, blank lines and trailing whitespace (Dolphin
        // and GTK differ in all of these).
        assert_eq!(
            parse_uri_list(b"# from a file manager\nfile:///tmp/a\n\n  file:///tmp/b  \n").unwrap(),
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
        assert_eq!(parse_uri_list(b"").unwrap(), Vec::<PathBuf>::new());
        assert!(matches!(
            parse_uri_list(b"file:///tmp/a\r\nhttps://example.com/b\r\n"),
            Err(ParseError::NotLocalFile(_))
        ));
        assert_eq!(parse_uri_list(b"\xff\xfe"), Err(ParseError::NotText));
    }

    #[test]
    fn gnome_copied_files_match_nautilus_byte_for_byte() {
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/c")];
        assert_eq!(
            format_gnome_copied_files(&paths, true),
            "cut\nfile:///tmp/a%20b\nfile:///tmp/c"
        );
        assert_eq!(
            format_gnome_copied_files(&paths, false),
            "copy\nfile:///tmp/a%20b\nfile:///tmp/c"
        );
    }

    #[test]
    fn gnome_copied_files_parse_cut_and_copy() {
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/c")];
        for cut in [true, false] {
            let text = format_gnome_copied_files(&paths, cut);
            assert_eq!(
                parse_gnome_copied_files(text.as_bytes()).unwrap(),
                FileList {
                    paths: paths.clone(),
                    cut
                }
            );
        }
        // Thunar ends with a newline; some writers use CRLF.
        assert_eq!(
            parse_gnome_copied_files(b"cut\r\nfile:///tmp/x\r\n").unwrap(),
            FileList {
                paths: vec![PathBuf::from("/tmp/x")],
                cut: true
            }
        );
        assert_eq!(
            parse_gnome_copied_files(b"link\nfile:///tmp/x"),
            Err(ParseError::UnknownAction("link".into()))
        );
        assert!(matches!(
            parse_gnome_copied_files(b"copy\nhttps://example.com/x"),
            Err(ParseError::NotLocalFile(_))
        ));
    }

    #[test]
    fn kde_cut_selection_is_one() {
        assert!(parse_kde_cut_selection(b"1"));
        assert!(parse_kde_cut_selection(b"1\n"));
        assert!(!parse_kde_cut_selection(b"0"));
        assert!(!parse_kde_cut_selection(b""));
    }
}
