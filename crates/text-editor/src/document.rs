use std::borrow::Cow;
use std::fmt;

pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineEnding {
    None,
    #[default]
    Lf,
    CrLf,
    Cr,
    Mixed,
}

impl LineEnding {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "No line endings",
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
            Self::Mixed => "Mixed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextFormat {
    pub(crate) encoding: TextEncoding,
    pub(crate) source_line_ending: LineEnding,
    pub(crate) save_line_ending: LineEnding,
}

impl Default for TextFormat {
    fn default() -> Self {
        Self {
            encoding: TextEncoding::Utf8,
            source_line_ending: LineEnding::None,
            save_line_ending: LineEnding::Lf,
        }
    }
}

impl TextFormat {
    pub(crate) fn status_against(self, saved: Self) -> String {
        let encoding_changed = self.encoding != saved.encoding;
        let encoding = if !encoding_changed {
            self.encoding.label().to_string()
        } else {
            format!("{} → {}", saved.encoding.label(), self.encoding.label(),)
        };
        let source_line_ending = match self.source_line_ending {
            LineEnding::None => saved.save_line_ending,
            source => source,
        };
        let line_ending_changed = source_line_ending != self.save_line_ending;
        let line_ending = if source_line_ending == LineEnding::Mixed || line_ending_changed {
            format!(
                "{} → {}",
                source_line_ending.label(),
                self.save_line_ending.label()
            )
        } else {
            self.save_line_ending.label().to_string()
        };
        let pending =
            if encoding_changed || line_ending_changed || source_line_ending == LineEnding::Mixed {
                " on save"
            } else {
                ""
            };
        format!("{encoding} · {line_ending}{pending}")
    }
}

pub(crate) fn has_unsaved_changes<T: PartialEq + ?Sized>(
    value: &T,
    saved_value: &T,
    format: TextFormat,
    saved_format: TextFormat,
) -> bool {
    value != saved_value || format != saved_format
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecodedDocument {
    pub(crate) text: String,
    pub(crate) format: TextFormat,
    pub(crate) original_bytes: Vec<u8>,
    /// Byte length of the longest line of `text`, measured while decoding
    /// so the caller can choose a view without scanning the text again.
    pub(crate) longest_line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodecError {
    TooLarge,
    UnsupportedUtf32,
    InvalidUtf8,
    InvalidUtf16,
    EncodedTooLarge,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "the document exceeds the 64 MiB safety limit",
            Self::UnsupportedUtf32 => "UTF-32 documents are not supported",
            Self::InvalidUtf8 => "the document is not valid UTF-8 or BOM-marked UTF-16",
            Self::InvalidUtf16 => "the UTF-16 document is malformed",
            Self::EncodedTooLarge => "the encoded document exceeds the 64 MiB safety limit",
        })
    }
}

impl std::error::Error for CodecError {}

pub(crate) fn decode(bytes: Vec<u8>) -> Result<DecodedDocument, CodecError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(CodecError::TooLarge);
    }
    if bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) || bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00])
    {
        return Err(CodecError::UnsupportedUtf32);
    }

    let (encoding, decoded) = if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        (
            TextEncoding::Utf8Bom,
            std::str::from_utf8(body)
                .map_err(|_| CodecError::InvalidUtf8)?
                .to_string(),
        )
    } else if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        (TextEncoding::Utf16Le, decode_utf16(body, true)?)
    } else if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        (TextEncoding::Utf16Be, decode_utf16(body, false)?)
    } else {
        (
            TextEncoding::Utf8,
            std::str::from_utf8(&bytes)
                .map_err(|_| CodecError::InvalidUtf8)?
                .to_string(),
        )
    };
    let normalized = normalize_owned_line_endings(decoded);
    let save_line_ending = preferred_line_ending(&normalized);
    let longest_line = crate::long_lines::longest_line_bytes(&normalized.text);
    Ok(DecodedDocument {
        longest_line,
        text: normalized.text,
        format: TextFormat {
            encoding,
            source_line_ending: normalized.kind,
            save_line_ending,
        },
        original_bytes: bytes,
    })
}

pub(crate) fn encode(text: &str, format: TextFormat) -> Result<Vec<u8>, CodecError> {
    // The editor keeps text normalized to LF, so the usual save borrows it
    // and the only full copy is the encoded output.
    let normalized = if text.contains('\r') {
        Cow::Owned(normalize_line_endings(text).text)
    } else {
        Cow::Borrowed(text)
    };
    let line_ending = match format.save_line_ending {
        LineEnding::CrLf => "\r\n",
        LineEnding::Cr => "\r",
        LineEnding::None | LineEnding::Lf | LineEnding::Mixed => "\n",
    };
    let serialized = if line_ending == "\n" {
        normalized
    } else {
        Cow::Owned(normalized.replace('\n', line_ending))
    };
    let output = match format.encoding {
        TextEncoding::Utf8 => serialized.into_owned().into_bytes(),
        TextEncoding::Utf8Bom => {
            let mut output = Vec::with_capacity(serialized.len().saturating_add(3));
            output.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            output.extend_from_slice(serialized.as_bytes());
            output
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let mut output =
                Vec::with_capacity(serialized.len().saturating_mul(2).saturating_add(2));
            output.extend_from_slice(if format.encoding == TextEncoding::Utf16Le {
                &[0xff, 0xfe]
            } else {
                &[0xfe, 0xff]
            });
            for unit in serialized.encode_utf16() {
                let bytes = if format.encoding == TextEncoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                output.extend_from_slice(&bytes);
                if output.len() > MAX_DOCUMENT_BYTES {
                    return Err(CodecError::EncodedTooLarge);
                }
            }
            output
        }
    };
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(CodecError::EncodedTooLarge);
    }
    Ok(output)
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, CodecError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(CodecError::InvalidUtf16);
    }
    let units = bytes.chunks_exact(2).map(|bytes| {
        let pair = [bytes[0], bytes[1]];
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| CodecError::InvalidUtf16)
}

struct NormalizedText {
    text: String,
    kind: LineEnding,
    lf: usize,
    crlf: usize,
    cr: usize,
    first: Option<LineEnding>,
}

/// [`normalize_line_endings`] for decoded text the caller no longer needs:
/// text without a carriage return, the common case, is moved rather than
/// copied.
fn normalize_owned_line_endings(text: String) -> NormalizedText {
    if text.as_bytes().contains(&b'\r') {
        return normalize_line_endings(&text);
    }
    let lf = text.bytes().filter(|&byte| byte == b'\n').count();
    NormalizedText {
        text,
        kind: if lf > 0 {
            LineEnding::Lf
        } else {
            LineEnding::None
        },
        lf,
        crlf: 0,
        cr: 0,
        first: (lf > 0).then_some(LineEnding::Lf),
    }
}

fn normalize_line_endings(text: &str) -> NormalizedText {
    let mut normalized = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut lf = 0_usize;
    let mut crlf = 0_usize;
    let mut cr = 0_usize;
    let mut first = None;
    while let Some(character) = characters.next() {
        match character {
            '\r' if characters.peek() == Some(&'\n') => {
                characters.next();
                crlf += 1;
                first.get_or_insert(LineEnding::CrLf);
                normalized.push('\n');
            }
            '\r' => {
                cr += 1;
                first.get_or_insert(LineEnding::Cr);
                normalized.push('\n');
            }
            '\n' => {
                lf += 1;
                first.get_or_insert(LineEnding::Lf);
                normalized.push('\n');
            }
            _ => normalized.push(character),
        }
    }
    let kinds = usize::from(lf > 0) + usize::from(crlf > 0) + usize::from(cr > 0);
    let kind = match kinds {
        0 => LineEnding::None,
        1 if lf > 0 => LineEnding::Lf,
        1 if crlf > 0 => LineEnding::CrLf,
        1 => LineEnding::Cr,
        _ => LineEnding::Mixed,
    };
    NormalizedText {
        text: normalized,
        kind,
        lf,
        crlf,
        cr,
        first,
    }
}

fn preferred_line_ending(normalized: &NormalizedText) -> LineEnding {
    if normalized.kind != LineEnding::Mixed {
        return match normalized.kind {
            LineEnding::None => LineEnding::Lf,
            kind => kind,
        };
    }
    let maximum = normalized.lf.max(normalized.crlf).max(normalized.cr);
    [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr]
        .into_iter()
        .find(|kind| {
            let count = match kind {
                LineEnding::Lf => normalized.lf,
                LineEnding::CrLf => normalized.crlf,
                LineEnding::Cr => normalized.cr,
                LineEnding::None | LineEnding::Mixed => 0,
            };
            count == maximum && normalized.first == Some(*kind)
        })
        .or_else(|| {
            [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr]
                .into_iter()
                .find(|kind| match kind {
                    LineEnding::Lf => normalized.lf == maximum,
                    LineEnding::CrLf => normalized.crlf == maximum,
                    LineEnding::Cr => normalized.cr == maximum,
                    LineEnding::None | LineEnding::Mixed => false,
                })
        })
        .unwrap_or(LineEnding::Lf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_bom_and_crlf_round_trip_with_normalized_editing_text() {
        let bytes = b"\xef\xbb\xbfhello\r\nworld\r\n".to_vec();
        let decoded = decode(bytes.clone()).unwrap();
        assert_eq!(decoded.text, "hello\nworld\n");
        assert_eq!(decoded.format.encoding, TextEncoding::Utf8Bom);
        assert_eq!(decoded.format.source_line_ending, LineEnding::CrLf);
        assert_eq!(encode(&decoded.text, decoded.format).unwrap(), bytes);
    }

    #[test]
    fn utf16_endianness_and_supplementary_characters_round_trip() {
        for encoding in [TextEncoding::Utf16Le, TextEncoding::Utf16Be] {
            let format = TextFormat {
                encoding,
                source_line_ending: LineEnding::Lf,
                save_line_ending: LineEnding::Lf,
            };
            let bytes = encode("hello 🦀\n", format).unwrap();
            let decoded = decode(bytes.clone()).unwrap();
            assert_eq!(decoded.text, "hello 🦀\n");
            assert_eq!(decoded.format.encoding, encoding);
            assert_eq!(encode(&decoded.text, decoded.format).unwrap(), bytes);
        }
    }

    #[test]
    fn mixed_line_endings_are_explicit_and_choose_the_dominant_style() {
        let decoded = decode(b"one\r\ntwo\r\nthree\nfour\r".to_vec()).unwrap();
        assert_eq!(decoded.text, "one\ntwo\nthree\nfour\n");
        assert_eq!(decoded.format.source_line_ending, LineEnding::Mixed);
        assert_eq!(decoded.format.save_line_ending, LineEnding::CrLf);
        assert_eq!(
            decoded.format.status_against(decoded.format),
            "UTF-8 · Mixed → CRLF on save"
        );
    }

    #[test]
    fn pending_format_conversions_are_disclosed_before_save() {
        let saved = TextFormat {
            encoding: TextEncoding::Utf8,
            source_line_ending: LineEnding::Lf,
            save_line_ending: LineEnding::Lf,
        };
        let pending = TextFormat {
            encoding: TextEncoding::Utf16Le,
            save_line_ending: LineEnding::CrLf,
            ..saved
        };

        assert_eq!(
            pending.status_against(saved),
            "UTF-8 → UTF-16 LE · LF → CRLF on save"
        );
    }

    #[test]
    fn format_only_changes_are_unsaved_until_reverted_or_written() {
        let saved = TextFormat::default();
        let converted = TextFormat {
            encoding: TextEncoding::Utf16Be,
            save_line_ending: LineEnding::CrLf,
            ..saved
        };

        assert!(has_unsaved_changes("same", "same", converted, saved));
        assert!(!has_unsaved_changes("same", "same", saved, saved));
        assert!(has_unsaved_changes("changed", "same", saved, saved));
    }

    #[test]
    fn selected_encoding_and_line_ending_drive_exact_output() {
        let opened = decode(b"one\ntwo\n".to_vec()).unwrap();
        let selected = TextFormat {
            encoding: TextEncoding::Utf16Be,
            save_line_ending: LineEnding::CrLf,
            ..opened.format
        };

        let bytes = encode(&opened.text, selected).unwrap();
        assert_eq!(&bytes[..2], &[0xfe, 0xff]);
        let written = decode(bytes).unwrap();
        assert_eq!(written.text, "one\ntwo\n");
        assert_eq!(written.format.encoding, TextEncoding::Utf16Be);
        assert_eq!(written.format.source_line_ending, LineEnding::CrLf);
        assert_eq!(written.format.save_line_ending, LineEnding::CrLf);
    }

    #[test]
    fn invalid_unicode_and_utf32_fail_without_lossy_decoding() {
        assert_eq!(decode(vec![0xff]), Err(CodecError::InvalidUtf8));
        assert_eq!(
            decode(vec![0xff, 0xfe, 0x00]),
            Err(CodecError::InvalidUtf16)
        );
        assert_eq!(
            decode(vec![0xff, 0xfe, 0x00, 0x00]),
            Err(CodecError::UnsupportedUtf32)
        );
    }

    /// Journey 5's large fixture size: 24 MiB, no carriage returns. Loading
    /// keeps the original bytes (the revision a save must match) plus one
    /// decoded copy; line-ending normalization must not add another, and
    /// saving must not copy the text before encoding it.
    #[test]
    fn a_large_document_decodes_and_encodes_with_one_copy() {
        let size = 24 * 1024 * 1024;
        let bytes = "0123456789abcdef\n".repeat(size / 17).into_bytes();
        let length = bytes.len();
        let (decoded, peak) = crate::test_alloc::peak_heap_during(|| decode(bytes).unwrap());
        assert_eq!(decoded.text.len(), length);
        assert_eq!(decoded.longest_line, 16);
        assert!(peak <= length + length / 8, "decode held {peak} bytes");

        let (encoded, peak) =
            crate::test_alloc::peak_heap_during(|| encode(&decoded.text, decoded.format).unwrap());
        assert_eq!(encoded, decoded.original_bytes);
        assert!(peak <= length + length / 8, "encode held {peak} bytes");
    }

    #[test]
    fn documents_and_encoded_output_are_bounded() {
        assert_eq!(
            decode(vec![b'a'; MAX_DOCUMENT_BYTES + 1]),
            Err(CodecError::TooLarge)
        );
        let oversized = "é".repeat(MAX_DOCUMENT_BYTES / 2 + 1);
        assert_eq!(
            encode(&oversized, TextFormat::default()),
            Err(CodecError::EncodedTooLarge)
        );
    }
}
