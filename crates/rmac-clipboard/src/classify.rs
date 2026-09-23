//! Choosing what to record from a clipboard offer, and summarising it.

use std::path::{Path, PathBuf};

use crate::{Draft, Kind, MAX_ENTRY_BYTES, MAX_TEXT_BYTES, PREVIEW_CHARS, SENSITIVE_HINT_MIME};

const IMAGE_TYPES: [&str; 6] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
    "image/tiff",
];
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];
const URI_LIST: &str = "text/uri-list";

/// Whether the offer carries the password-manager hint whose value must be
/// read before anything else is fetched.
pub fn has_sensitivity_hint(types: &[String]) -> bool {
    types.iter().any(|mime| mime == SENSITIVE_HINT_MIME)
}

/// The hint's value is `secret` for passwords. Any other value is recorded.
pub fn is_secret_hint(value: &[u8]) -> bool {
    String::from_utf8_lossy(value).trim() == "secret"
}

/// The offered types worth reading, best first. Copied files come as a URI
/// list; an image is preferred over the HTML or text that browsers offer
/// beside it; plain text is last. A URI list of web links falls through to
/// text when `summarise` rejects it.
pub fn candidates(types: &[String]) -> Vec<(Kind, String)> {
    let offered = |wanted: &str| -> Option<String> {
        types
            .iter()
            .find(|mime| mime.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    let mut out = Vec::new();
    if let Some(mime) = offered(URI_LIST) {
        out.push((Kind::Files, mime));
    }
    if let Some(mime) = IMAGE_TYPES.iter().find_map(|wanted| offered(*wanted)) {
        out.push((Kind::Image, mime));
    }
    if let Some(mime) = TEXT_TYPES.iter().find_map(|wanted| offered(*wanted)) {
        out.push((Kind::Text, mime));
    }
    out
}

/// The largest payload worth reading for a candidate.
pub const fn byte_limit(kind: Kind) -> u64 {
    match kind {
        Kind::Text | Kind::Files => MAX_TEXT_BYTES,
        Kind::Image => MAX_ENTRY_BYTES,
    }
}

/// Summarise a payload for the list, or reject it (empty, oversized, not
/// UTF-8 text, or a URI list that is not only local files).
pub fn summarise(kind: Kind, mime: &str, bytes: &[u8]) -> Option<Draft> {
    if bytes.is_empty() || bytes.len() as u64 > byte_limit(kind) {
        return None;
    }
    let (title, detail) = match kind {
        Kind::Text => {
            let text = std::str::from_utf8(bytes).ok()?;
            let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
            (truncate(line, PREVIEW_CHARS), String::new())
        }
        Kind::Files => {
            let paths = file_paths(bytes)?;
            let first = &paths[0];
            let name = first
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| first.to_string_lossy().into_owned());
            let title = match paths.len() {
                1 => name,
                count => format!("{name} and {} more", count - 1),
            };
            let folder = first
                .parent()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            (title, folder)
        }
        Kind::Image => {
            let label = image_label(mime)?;
            let detail = png_dimensions(bytes)
                .map(|(width, height)| format!("{width} × {height}"))
                .unwrap_or_default();
            (label.to_owned(), detail)
        }
    };
    Some(Draft {
        kind,
        mime: mime.to_owned(),
        title,
        detail,
        size: bytes.len() as u64,
        digest: digest(kind, bytes),
    })
}

/// Local paths from a `text/uri-list`, or `None` unless every entry is a
/// local `file:` URI.
pub fn file_paths(bytes: &[u8]) -> Option<Vec<PathBuf>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let paths = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(file_uri_path)
        .collect::<Option<Vec<_>>>()?;
    (!paths.is_empty()).then_some(paths)
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let decoded = percent_decode(path)?;
    let path = PathBuf::from(String::from_utf8(decoded).ok()?);
    path.is_absolute().then_some(path)
}

fn percent_decode(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(out)
}

fn image_label(mime: &str) -> Option<&'static str> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "image/png" => "PNG Image",
        "image/jpeg" => "JPEG Image",
        "image/webp" => "WebP Image",
        "image/gif" => "GIF Image",
        "image/bmp" => "BMP Image",
        "image/tiff" => "TIFF Image",
        _ => return None,
    })
}

/// Width and height from a PNG's IHDR chunk, without decoding the image.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || bytes[12..16] != *b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

fn truncate(value: &str, limit: usize) -> String {
    match value.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_owned(),
    }
}

/// FNV-1a over the kind and payload, used to recognise a re-copy.
pub fn digest(kind: Kind, bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in kind.wire().as_bytes().iter().chain(bytes) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
