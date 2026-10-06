//! Rich flavours on the system clipboard.
//!
//! GPUI's clipboard item holds one string and an optional metadata string.
//! Rich copies carry their other flavours (RTF and HTML) in that metadata,
//! framed as below, and Lulo's Wayland backend offers each framed flavour
//! to other apps under its own MIME type next to the plain text (and frames
//! an RTF flavour another app offers the same way when it pastes here).
//! The framing must match `shell/compat/gpui_linux/src/linux/wayland/clipboard.rs`.
//!
//! ```text
//! x-lulo-clipboard-formats/1\n
//! <mime>\n<byte length>\n<payload>   (repeated)
//! ```

/// The first line of framed metadata.
pub const FORMATS_HEADER: &str = "x-lulo-clipboard-formats/1\n";
pub const RTF_MIME: &str = "text/rtf";
pub const HTML_MIME: &str = "text/html";

/// `formats` framed as clipboard metadata.
pub fn encode_formats(formats: &[(&str, &str)]) -> String {
    let mut out = String::from(FORMATS_HEADER);
    for (mime, payload) in formats {
        out.push_str(mime);
        out.push('\n');
        out.push_str(&payload.len().to_string());
        out.push('\n');
        out.push_str(payload);
    }
    out
}

/// The flavours framed in `metadata`, or none when it is not framed.
pub fn decode_formats(metadata: &str) -> Vec<(&str, &str)> {
    let Some(mut rest) = metadata.strip_prefix(FORMATS_HEADER) else {
        return Vec::new();
    };
    let mut formats = Vec::new();
    while !rest.is_empty() {
        let Some((mime, after)) = rest.split_once('\n') else {
            break;
        };
        let Some((length, after)) = after.split_once('\n') else {
            break;
        };
        let Ok(length) = length.parse::<usize>() else {
            break;
        };
        if length > after.len() || !after.is_char_boundary(length) {
            break;
        }
        formats.push((mime, &after[..length]));
        rest = &after[length..];
    }
    formats
}

/// The payload of `mime` in `metadata`, if framed there.
pub fn format<'a>(metadata: &'a str, mime: &str) -> Option<&'a str> {
    decode_formats(metadata)
        .into_iter()
        .find(|(candidate, _)| *candidate == mime)
        .map(|(_, payload)| payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_formats_round_trip_with_newlines_and_unicode() {
        let rtf = "{\\rtf1\nline\n}";
        let html = "<p>caf\u{e9}\n</p>";
        let metadata = encode_formats(&[(RTF_MIME, rtf), (HTML_MIME, html)]);
        assert_eq!(
            decode_formats(&metadata),
            [(RTF_MIME, rtf), (HTML_MIME, html)]
        );
        assert_eq!(format(&metadata, HTML_MIME), Some(html));
        assert_eq!(format(&metadata, "image/png"), None);
    }

    #[test]
    fn other_metadata_and_damaged_frames_decode_to_nothing() {
        assert!(decode_formats("{\"json\":true}").is_empty());
        let damaged = format!("{FORMATS_HEADER}text/rtf\n999\nshort");
        assert!(decode_formats(&damaged).is_empty());
    }
}
