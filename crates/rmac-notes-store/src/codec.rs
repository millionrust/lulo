use std::fmt;

use crate::{
    AttachmentId, AttachmentKind, AttachmentRecord, FolderId, FolderRecord, LibrarySnapshot,
    NoteId, NoteRecord, ValidationError, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE, MAX_FOLDERS,
    MAX_NOTES, MAX_TAGS_PER_NOTE,
};

const MAGIC: &[u8; 8] = b"RMNLIB\0\0";
pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_LIBRARY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    TooLarge,
    UnsupportedVersion,
    Malformed,
    Invalid(ValidationError),
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "the Notes library exceeds the 64 MiB metadata safety limit",
            Self::UnsupportedVersion => "the Notes library uses an unsupported schema version",
            Self::Malformed => "the Notes library record is malformed",
            Self::Invalid(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for CodecError {}

pub fn encode(snapshot: &LibrarySnapshot) -> Result<Vec<u8>, CodecError> {
    snapshot.validate().map_err(CodecError::Invalid)?;
    let mut folders = snapshot.folders.iter().collect::<Vec<_>>();
    let mut notes = snapshot.notes.iter().collect::<Vec<_>>();
    let mut attachments = snapshot.attachments.iter().collect::<Vec<_>>();
    folders.sort_by_key(|record| record.id);
    notes.sort_by_key(|record| record.id);
    attachments.sort_by_key(|record| record.id);

    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    put_u16(&mut output, SCHEMA_VERSION);
    put_u64(&mut output, snapshot.revision);
    put_u64(&mut output, snapshot.next_note_id);
    put_u64(&mut output, snapshot.next_folder_id);
    put_u64(&mut output, snapshot.next_attachment_id);
    put_count(&mut output, folders.len())?;
    put_count(&mut output, notes.len())?;
    put_count(&mut output, attachments.len())?;

    for folder in folders {
        put_u64(&mut output, folder.id.get());
        put_u64(&mut output, folder.revision);
        put_string(&mut output, &folder.name)?;
        put_bool(&mut output, folder.deleted);
    }
    for note in notes {
        put_u64(&mut output, note.id.get());
        put_u64(&mut output, note.revision);
        put_u64(&mut output, note.created_unix_ms);
        put_u64(&mut output, note.modified_unix_ms);
        put_string(&mut output, &note.title)?;
        put_string(&mut output, &note.body)?;
        put_count(&mut output, note.tags.len())?;
        for tag in &note.tags {
            put_string(&mut output, tag)?;
        }
        put_u64(
            &mut output,
            note.folder_id.map(FolderId::get).unwrap_or_default(),
        );
        put_bool(&mut output, note.pinned);
        put_bool(&mut output, note.deleted);
        put_count(&mut output, note.attachments.len())?;
        for attachment in &note.attachments {
            put_u64(&mut output, attachment.get());
        }
    }
    for attachment in attachments {
        put_u64(&mut output, attachment.id.get());
        put_u64(&mut output, attachment.revision);
        put_u64(&mut output, attachment.note_id.get());
        put_string(&mut output, &attachment.display_name)?;
        output.push(match attachment.kind {
            AttachmentKind::Png => 0,
            AttachmentKind::Jpeg => 1,
            AttachmentKind::Gif => 2,
            AttachmentKind::WebP => 3,
        });
        put_u64(&mut output, attachment.byte_len);
        output.extend_from_slice(&attachment.sha256);
        put_bool(&mut output, attachment.deleted);
    }
    if output.len() > MAX_LIBRARY_BYTES {
        return Err(CodecError::TooLarge);
    }
    Ok(output)
}

pub fn decode(bytes: &[u8]) -> Result<LibrarySnapshot, CodecError> {
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(CodecError::TooLarge);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(CodecError::Malformed);
    }
    if reader.u16()? != SCHEMA_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let revision = reader.u64()?;
    let next_note_id = reader.u64()?;
    let next_folder_id = reader.u64()?;
    let next_attachment_id = reader.u64()?;
    let folder_count = reader.count(MAX_FOLDERS)?;
    let note_count = reader.count(MAX_NOTES)?;
    let attachment_count = reader.count(MAX_ATTACHMENTS)?;

    let mut folders = Vec::with_capacity(folder_count);
    for _ in 0..folder_count {
        folders.push(FolderRecord {
            id: FolderId::new(reader.u64()?).ok_or(CodecError::Malformed)?,
            revision: reader.u64()?,
            name: reader.string()?,
            deleted: reader.boolean()?,
        });
    }
    let mut notes = Vec::with_capacity(note_count);
    for _ in 0..note_count {
        let id = NoteId::new(reader.u64()?).ok_or(CodecError::Malformed)?;
        let note_revision = reader.u64()?;
        let created_unix_ms = reader.u64()?;
        let modified_unix_ms = reader.u64()?;
        let title = reader.string()?;
        let body = reader.string()?;
        let tag_count = reader.count(MAX_TAGS_PER_NOTE)?;
        let mut tags = Vec::with_capacity(tag_count);
        for _ in 0..tag_count {
            tags.push(reader.string()?);
        }
        let raw_folder = reader.u64()?;
        let folder_id = if raw_folder == 0 {
            None
        } else {
            Some(FolderId::new(raw_folder).ok_or(CodecError::Malformed)?)
        };
        let pinned = reader.boolean()?;
        let deleted = reader.boolean()?;
        let attachment_ref_count = reader.count(MAX_ATTACHMENTS_PER_NOTE)?;
        let mut attachment_refs = Vec::with_capacity(attachment_ref_count);
        for _ in 0..attachment_ref_count {
            attachment_refs.push(AttachmentId::new(reader.u64()?).ok_or(CodecError::Malformed)?);
        }
        notes.push(NoteRecord {
            id,
            revision: note_revision,
            created_unix_ms,
            modified_unix_ms,
            title,
            body,
            tags,
            folder_id,
            pinned,
            deleted,
            attachments: attachment_refs,
        });
    }
    let mut attachments = Vec::with_capacity(attachment_count);
    for _ in 0..attachment_count {
        let id = AttachmentId::new(reader.u64()?).ok_or(CodecError::Malformed)?;
        let attachment_revision = reader.u64()?;
        let note_id = NoteId::new(reader.u64()?).ok_or(CodecError::Malformed)?;
        let display_name = reader.string()?;
        let kind = match reader.byte()? {
            0 => AttachmentKind::Png,
            1 => AttachmentKind::Jpeg,
            2 => AttachmentKind::Gif,
            3 => AttachmentKind::WebP,
            _ => return Err(CodecError::Malformed),
        };
        let byte_len = reader.u64()?;
        let sha256 = reader
            .take(32)?
            .try_into()
            .map_err(|_| CodecError::Malformed)?;
        let deleted = reader.boolean()?;
        attachments.push(AttachmentRecord {
            id,
            revision: attachment_revision,
            note_id,
            display_name,
            kind,
            byte_len,
            sha256,
            deleted,
        });
    }
    if !reader.is_empty() {
        return Err(CodecError::Malformed);
    }
    let snapshot = LibrarySnapshot {
        revision,
        next_note_id,
        next_folder_id,
        next_attachment_id,
        folders,
        notes,
        attachments,
    };
    snapshot.validate().map_err(CodecError::Invalid)?;
    Ok(snapshot)
}

fn put_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_bool(output: &mut Vec<u8>, value: bool) {
    output.push(u8::from(value));
}

fn put_count(output: &mut Vec<u8>, value: usize) -> Result<(), CodecError> {
    let value = u32::try_from(value).map_err(|_| CodecError::TooLarge)?;
    output.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_string(output: &mut Vec<u8>, value: &str) -> Result<(), CodecError> {
    put_count(output, value.len())?;
    output.extend_from_slice(value.as_bytes());
    if output.len() > MAX_LIBRARY_BYTES {
        return Err(CodecError::TooLarge);
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(CodecError::Malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(CodecError::Malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, CodecError> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn boolean(&mut self) -> Result<bool, CodecError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CodecError::Malformed),
        }
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| CodecError::Malformed)
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        self.take(4)?
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| CodecError::Malformed)
    }

    fn u64(&mut self) -> Result<u64, CodecError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| CodecError::Malformed)
    }

    fn count(&mut self, maximum: usize) -> Result<usize, CodecError> {
        let value = usize::try_from(self.u32()?).map_err(|_| CodecError::Malformed)?;
        (value <= maximum)
            .then_some(value)
            .ok_or(CodecError::Malformed)
    }

    fn string(&mut self) -> Result<String, CodecError> {
        let length = self.count(MAX_LIBRARY_BYTES)?;
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| CodecError::Malformed)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    #[test]
    fn versioned_snapshot_round_trips_unicode_exactly() {
        let snapshot = fixture();
        let bytes = encode(&snapshot).unwrap();
        assert_eq!(&bytes[..MAGIC.len()], MAGIC);
        assert_eq!(decode(&bytes).unwrap(), snapshot);
    }

    #[test]
    fn encoding_is_canonical_across_record_order() {
        let mut snapshot = fixture();
        let folder_id = FolderId::new(2).unwrap();
        snapshot.folders.push(FolderRecord {
            id: folder_id,
            revision: 1,
            name: "Archive".into(),
            deleted: false,
        });
        snapshot.notes.push(NoteRecord {
            id: NoteId::new(2).unwrap(),
            revision: 1,
            created_unix_ms: 30,
            modified_unix_ms: 30,
            title: "Second".into(),
            body: String::new(),
            tags: Vec::new(),
            folder_id: Some(folder_id),
            pinned: false,
            deleted: false,
            attachments: Vec::new(),
        });
        snapshot.next_folder_id = 3;
        snapshot.next_note_id = 3;
        let mut reordered = snapshot.clone();
        reordered.folders.reverse();
        reordered.notes.reverse();
        reordered.attachments.reverse();
        assert_eq!(encode(&snapshot).unwrap(), encode(&reordered).unwrap());
    }

    #[test]
    fn truncated_trailing_and_future_records_fail_closed() {
        let bytes = encode(&fixture()).unwrap();
        for length in [0, MAGIC.len(), bytes.len() - 1] {
            assert!(decode(&bytes[..length]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(decode(&trailing), Err(CodecError::Malformed));

        let mut future = bytes;
        future[MAGIC.len()..MAGIC.len() + 2].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(decode(&future), Err(CodecError::UnsupportedVersion));
    }

    #[test]
    fn invalid_candidates_are_never_encoded() {
        let mut snapshot = fixture();
        snapshot.attachments[0].sha256 = [0; 32];
        assert_eq!(
            encode(&snapshot),
            Err(CodecError::Invalid(ValidationError::InvalidAttachment))
        );
    }

    #[test]
    fn excessive_input_is_rejected_before_parsing() {
        assert_eq!(
            decode(&vec![0; MAX_LIBRARY_BYTES + 1]),
            Err(CodecError::TooLarge)
        );
    }
}
