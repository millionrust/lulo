use std::fmt;

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
/// Smart Folders stored with the library (File ▸ New Smart Folder).
pub const MAX_SMART_FOLDERS: usize = 1_000;
/// Locked-note password generations kept so notes locked under an older
/// password (Notes ▸ Settings… ▸ Reset Password…) stay openable with it.
pub const MAX_LOCK_KEYS: usize = 64;
pub const MAX_LOCK_HINT_BYTES: usize = 256;
/// Sealed note payload ceiling: the plaintext limits plus generous framing.
pub const MAX_SEALED_NOTE_BYTES: usize = MAX_BODY_BYTES + 1024 * 1024;
/// The durable display name every sealed attachment record carries; the real
/// name lives inside its note's sealed payload.
pub const SEALED_ATTACHMENT_NAME: &str = "Locked Attachment";

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
stable_id!(SmartFolderId);

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
    /// `Some(key id)` when the managed bytes are sealed with that locked-note
    /// key: `byte_len`/`sha256` then describe the sealed file, and the real
    /// display name is inside the owning note's sealed payload.
    pub sealed_key: Option<u32>,
}

/// One XChaCha20-Poly1305 message: a random 192-bit nonce and the ciphertext
/// with its 16-byte authentication tag appended.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedBlob {
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for SealedBlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedBlob")
            .field("ciphertext_bytes", &self.ciphertext.len())
            .finish()
    }
}

/// A locked note's encrypted title, body, tags and attachment names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteLock {
    pub key_id: u32,
    pub sealed: SealedBlob,
}

/// Argon2id cost parameters recorded with each locked-note password so a
/// later release can raise them without breaking older keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockKdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

/// One locked-note password generation: its per-password random salt, KDF
/// parameters, a verifier sealed with the derived key, and the plaintext
/// hint the person chose (macOS Notes shows it after a wrong password).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockKeyRecord {
    pub id: u32,
    pub salt: [u8; 16],
    pub kdf: LockKdfParams,
    pub verifier: SealedBlob,
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub revision: u64,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
    /// Empty at rest while `lock` is set; the sealed payload holds it.
    pub title: String,
    /// Empty at rest while `lock` is set; the sealed payload holds it.
    pub body: String,
    /// Empty at rest while `lock` is set; the sealed payload holds them.
    pub tags: Vec<String>,
    pub folder_id: Option<FolderId>,
    pub pinned: bool,
    pub deleted: bool,
    pub attachments: Vec<AttachmentId>,
    pub lock: Option<NoteLock>,
}

/// File ▸ New Smart Folder: a saved, self-updating collection of every note
/// with one tag, stored with the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartFolderRecord {
    pub id: SmartFolderId,
    pub name: String,
    pub tag: String,
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
    pub next_smart_folder_id: u64,
    pub smart_folders: Vec<SmartFolderRecord>,
    pub next_lock_key_id: u32,
    /// The password new locks use; older generations stay in `lock_keys`
    /// while any note or attachment is still sealed with them.
    pub current_lock_key: Option<u32>,
    pub lock_keys: Vec<LockKeyRecord>,
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
            next_smart_folder_id: 1,
            smart_folders: Vec::new(),
            next_lock_key_id: 1,
            current_lock_key: None,
            lock_keys: Vec::new(),
        }
    }
}

impl LibrarySnapshot {
    pub fn lock_key(&self, id: u32) -> Option<&LockKeyRecord> {
        self.lock_keys.iter().find(|record| record.id == id)
    }
}
