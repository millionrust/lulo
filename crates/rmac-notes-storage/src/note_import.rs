use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::Path;

use rmac_notes_store::{FolderId, NewNote, MAX_BODY_BYTES, MAX_TITLE_BYTES};
use rmac_storage::{Backend, FileSystem};

pub const MAX_IMPORTED_TEXT_SOURCE_BYTES: usize = MAX_BODY_BYTES * 2 + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportedTextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl ImportedTextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextImportError {
    Io(io::ErrorKind),
    TooLarge,
    UnsupportedUtf32,
    InvalidUtf8,
    InvalidUtf16,
    InvalidText,
}

impl fmt::Display for TextImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "Notes could not read the selected text file",
            Self::TooLarge => "The selected text file exceeds the Notes import safety limit",
            Self::UnsupportedUtf32 => "Notes does not import UTF-32 text files",
            Self::InvalidUtf8 => "The selected file is not valid UTF-8 or BOM-marked UTF-16",
            Self::InvalidUtf16 => "The selected UTF-16 text file is malformed",
            Self::InvalidText => "The selected file contains text Notes cannot store safely",
        })
    }
}

impl std::error::Error for TextImportError {}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedTextNote {
    title: String,
    body: String,
    encoding: ImportedTextEncoding,
    source_byte_len: u64,
}

impl fmt::Debug for PreparedTextNote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTextNote")
            .field("title", &"<redacted>")
            .field("body", &"<redacted>")
            .field("encoding", &self.encoding)
            .field("source_byte_len", &self.source_byte_len)
            .field("decoded_body_bytes", &self.body.len())
            .finish()
    }
}

impl PreparedTextNote {
    pub fn encoding(&self) -> ImportedTextEncoding {
        self.encoding
    }

    pub fn source_byte_len(&self) -> u64 {
        self.source_byte_len
    }

    pub fn decoded_body_bytes(&self) -> usize {
        self.body.len()
    }

    pub fn new_note(&self, created_unix_ms: u64, folder_id: Option<FolderId>) -> NewNote {
        NewNote {
            created_unix_ms,
            title: self.title.clone(),
            body: self.body.clone(),
            tags: Vec::new(),
            folder_id,
        }
    }
}

pub fn prepare_text_note(path: &Path) -> Result<PreparedTextNote, TextImportError> {
    prepare_text_note_with_backend(path, &FileSystem)
}

pub(crate) fn prepare_text_note_with_backend<B: Backend>(
    path: &Path,
    backend: &B,
) -> Result<PreparedTextNote, TextImportError> {
    let bytes = backend
        .read_bounded(path, MAX_IMPORTED_TEXT_SOURCE_BYTES)
        .map_err(|error| match error.kind() {
            io::ErrorKind::InvalidData => TextImportError::TooLarge,
            kind => TextImportError::Io(kind),
        })?;
    let source_byte_len = bytes.len() as u64;
    let (encoding, body) = decode(bytes)?;
    if body.len() > MAX_BODY_BYTES || body.contains('\0') {
        return Err(if body.len() > MAX_BODY_BYTES {
            TextImportError::TooLarge
        } else {
            TextImportError::InvalidText
        });
    }
    Ok(PreparedTextNote {
        title: import_title(path.file_stem()),
        body,
        encoding,
        source_byte_len,
    })
}

fn decode(bytes: Vec<u8>) -> Result<(ImportedTextEncoding, String), TextImportError> {
    if bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) || bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00])
    {
        return Err(TextImportError::UnsupportedUtf32);
    }
    if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return std::str::from_utf8(body)
            .map(|text| (ImportedTextEncoding::Utf8Bom, text.to_string()))
            .map_err(|_| TextImportError::InvalidUtf8);
    }
    if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return decode_utf16(body, true).map(|text| (ImportedTextEncoding::Utf16Le, text));
    }
    if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return decode_utf16(body, false).map(|text| (ImportedTextEncoding::Utf16Be, text));
    }
    std::str::from_utf8(&bytes)
        .map(|text| (ImportedTextEncoding::Utf8, text.to_string()))
        .map_err(|_| TextImportError::InvalidUtf8)
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, TextImportError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(TextImportError::InvalidUtf16);
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
        .map_err(|_| TextImportError::InvalidUtf16)
}

fn import_title(stem: Option<&OsStr>) -> String {
    let stem = stem
        .and_then(OsStr::to_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .unwrap_or("Imported Note");
    let end = stem
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(stem.len()))
        .take_while(|index| *index <= MAX_TITLE_BYTES)
        .last()
        .unwrap_or_default();
    let title = &stem[..end];
    if title.is_empty() {
        "Imported Note".into()
    } else {
        title.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeBackend(Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>);

    impl FakeBackend {
        fn set(&self, path: &Path, bytes: Vec<u8>) {
            self.0.lock().unwrap().insert(path.to_path_buf(), bytes);
        }
    }

    impl Backend for FakeBackend {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
        let mut bytes = if little_endian {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in text.encode_utf16() {
            let pair = if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&pair);
        }
        bytes
    }

    #[test]
    fn supported_encodings_preserve_unicode_and_line_endings_exactly() {
        let backend = FakeBackend::default();
        let path = Path::new("/portal/Private Plan.md");
        let cases = [
            (
                ImportedTextEncoding::Utf8,
                "hello 🦀\r\n".as_bytes().to_vec(),
            ),
            (
                ImportedTextEncoding::Utf8Bom,
                [b"\xef\xbb\xbf".as_slice(), "hello 🦀\r\n".as_bytes()].concat(),
            ),
            (ImportedTextEncoding::Utf16Le, utf16("hello 🦀\r\n", true)),
            (ImportedTextEncoding::Utf16Be, utf16("hello 🦀\r\n", false)),
        ];
        for (encoding, bytes) in cases {
            backend.set(path, bytes.clone());
            let prepared = prepare_text_note_with_backend(path, &backend).unwrap();
            assert_eq!(prepared.encoding(), encoding);
            assert_eq!(prepared.source_byte_len(), bytes.len() as u64);
            let note = prepared.new_note(10, None);
            assert_eq!(note.title, "Private Plan");
            assert_eq!(note.body, "hello 🦀\r\n");
        }
    }

    #[test]
    fn malformed_unsupported_and_nul_text_fail_without_lossy_replacement() {
        let backend = FakeBackend::default();
        let path = Path::new("/portal/private.txt");
        for (bytes, expected) in [
            (vec![0xff], TextImportError::InvalidUtf8),
            (vec![0xff, 0xfe, 0x00], TextImportError::InvalidUtf16),
            (
                vec![0xff, 0xfe, 0x00, 0x00],
                TextImportError::UnsupportedUtf32,
            ),
            (b"private\0text".to_vec(), TextImportError::InvalidText),
        ] {
            backend.set(path, bytes);
            assert_eq!(
                prepare_text_note_with_backend(path, &backend),
                Err(expected)
            );
        }
    }

    #[test]
    fn prepared_debug_redacts_title_body_and_source_path() {
        let backend = FakeBackend::default();
        let path = Path::new("/portal/private-title.md");
        backend.set(path, b"private body".to_vec());
        let prepared = prepare_text_note_with_backend(path, &backend).unwrap();
        let debug = format!("{prepared:?}");
        assert!(!debug.contains("private-title"));
        assert!(!debug.contains("private body"));
        assert!(!debug.contains("/portal"));
    }
}
