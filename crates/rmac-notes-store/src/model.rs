pub const MAX_NOTES: usize = 100_000;
pub const MAX_FOLDERS: usize = 10_000;
pub const MAX_ATTACHMENTS: usize = 200_000;
pub const MAX_TAGS_PER_NOTE: usize = 32;
pub const MAX_ATTACHMENTS_PER_NOTE: usize = 128;
pub const MAX_TITLE_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_NAME_BYTES: usize = 1024;
pub const MAX_TAG_BYTES: usize = 256;
pub const MAX_ATTACHMENT_BYTES: u64 = 256 * 1024 * 1024;

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub fn new(value: u64) -> Option<Self> {
                (value != 0).then_some(Self(value))
            }

            pub fn get(self) -> u64 {
                self.0
            }
        }
    };
}

stable_id!(NoteId);
stable_id!(FolderId);
stable_id!(AttachmentId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Png,
    Jpeg,
    Gif,
    WebP,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Edited,
    Created,
    Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRecord {
    pub id: FolderId,
    pub revision: u64,
    pub name: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentRecord {
    pub id: AttachmentId,
    pub revision: u64,
    pub note_id: NoteId,
    pub display_name: String,
    pub kind: AttachmentKind,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub revision: u64,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub folder_id: Option<FolderId>,
    pub pinned: bool,
    pub deleted: bool,
    pub attachments: Vec<AttachmentId>,
}

/// Complete authoritative library snapshot. Vector order is not identity or
/// list order; encoding canonicalizes records by stable ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibrarySnapshot {
    pub revision: u64,
    pub sort_order: SortOrder,
    pub next_note_id: u64,
    pub next_folder_id: u64,
    pub next_attachment_id: u64,
    pub folders: Vec<FolderRecord>,
    pub notes: Vec<NoteRecord>,
    pub attachments: Vec<AttachmentRecord>,
}

impl Default for LibrarySnapshot {
    fn default() -> Self {
        Self {
            revision: 1,
            sort_order: SortOrder::Edited,
            next_note_id: 1,
            next_folder_id: 1,
            next_attachment_id: 1,
            folders: Vec::new(),
            notes: Vec::new(),
            attachments: Vec::new(),
        }
    }
}
