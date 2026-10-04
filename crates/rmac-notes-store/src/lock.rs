//! Locked-note encryption (File ▸ Lock Note).
//!
//! A password is stretched with Argon2id under a random per-password salt
//! into a 256-bit key; every locked note's title, body, tags and attachment
//! names, and every attachment of a locked note, is sealed with
//! XChaCha20-Poly1305 under a fresh random 192-bit nonce. The associated data
//! binds each message to its key generation and record identity, so sealed
//! bytes cannot be swapped between notes, attachments or passwords without
//! failing authentication. Keys and decrypted plaintext are zeroized on drop.

use std::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    AttachmentId, LockKdfParams, LockKeyRecord, NoteId, NoteLock, SealedBlob,
    MAX_ATTACHMENTS_PER_NOTE, MAX_LOCK_HINT_BYTES, MAX_SEALED_NOTE_BYTES, MAX_TAGS_PER_NOTE,
};

pub const LOCK_SALT_BYTES: usize = 16;
pub const LOCK_NONCE_BYTES: usize = 24;
pub const LOCK_TAG_BYTES: usize = 16;
pub const LOCK_KEY_BYTES: usize = 32;
/// Bytes a sealed attachment file adds to its plaintext: nonce and tag.
pub const SEALED_ATTACHMENT_OVERHEAD: u64 = (LOCK_NONCE_BYTES + LOCK_TAG_BYTES) as u64;
pub(crate) const VERIFIER_PLAINTEXT: &[u8; 32] = b"rmac notes locked-note verifier1";
pub(crate) const VERIFIER_CIPHERTEXT_BYTES: usize = VERIFIER_PLAINTEXT.len() + LOCK_TAG_BYTES;

/// Argon2id parameters for new passwords: 19 MiB, two passes, one lane (the
/// OWASP baseline). Measured on the four-thread reference laptop well under
/// the 250 ms budget, and it never needs more than ~19 MiB of memory.
pub const DEFAULT_LOCK_KDF: LockKdfParams = LockKdfParams {
    memory_kib: 19 * 1024,
    iterations: 2,
    parallelism: 1,
};

/// Bounds a stored record must respect, so a corrupted or hostile library
/// can never make unlocking allocate or compute without limit.
pub const MIN_LOCK_KDF_MEMORY_KIB: u32 = 8;
pub const MAX_LOCK_KDF_MEMORY_KIB: u32 = 256 * 1024;
pub const MAX_LOCK_KDF_ITERATIONS: u32 = 16;
pub const MAX_LOCK_KDF_PARALLELISM: u32 = 8;

const PAYLOAD_VERSION: u8 = 1;
const NOTE_AAD: &[u8] = b"rmac-notes/locked-note/v1";
const ATTACHMENT_AAD: &[u8] = b"rmac-notes/locked-attachment/v1";
const VERIFIER_AAD: &[u8] = b"rmac-notes/lock-verifier/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockError {
    /// The password does not open this key generation.
    WrongPassword,
    /// Sealed bytes failed authentication: changed on disk or swapped.
    Tampered,
    /// The sealed bytes belong to a different key generation.
    WrongKey,
    EmptyPassword,
    InvalidParameters,
    InvalidHint,
    TooLarge,
    Malformed,
    /// The operating system's random source failed.
    Random,
}

impl fmt::Display for LockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongPassword => "The password is incorrect",
            Self::Tampered => "A locked note's encrypted data was changed or damaged",
            Self::WrongKey => "A locked note uses a different password",
            Self::EmptyPassword => "Enter a password",
            Self::InvalidParameters => "A locked-note password record is invalid",
            Self::InvalidHint => "The password hint is too long or contains control characters",
            Self::TooLarge => "A locked note exceeds a Notes safety limit",
            Self::Malformed => "A locked note's decrypted data is malformed",
            Self::Random => "Notes could not get secure random bytes from the system",
        })
    }
}

impl std::error::Error for LockError {}

/// A derived locked-note key. Never persisted; zeroized on drop.
pub struct LockKey {
    key_id: u32,
    bytes: Zeroizing<[u8; LOCK_KEY_BYTES]>,
}

impl LockKey {
    pub fn key_id(&self) -> u32 {
        self.key_id
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(self.bytes.as_slice())
            .expect("a 256-bit key is the XChaCha20-Poly1305 key size")
    }

    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<SealedBlob, LockError> {
        let mut nonce = [0_u8; LOCK_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| LockError::Random)?;
        let ciphertext = self
            .cipher()
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| LockError::TooLarge)?;
        Ok(SealedBlob { nonce, ciphertext })
    }

    fn open(&self, aad: &[u8], sealed: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, LockError> {
        self.cipher()
            .decrypt(
                &XNonce::from(sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| LockError::Tampered)
    }
}

impl fmt::Debug for LockKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockKey")
            .field("key_id", &self.key_id)
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// The plaintext a locked note seals. Zeroized on drop.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LockedNoteContent {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    /// Real display names of the note's sealed attachments, by identity.
    pub attachment_names: Vec<(AttachmentId, String)>,
}

impl LockedNoteContent {
    pub fn attachment_name(&self, id: AttachmentId) -> Option<&str> {
        self.attachment_names
            .iter()
            .find(|(attachment, _)| *attachment == id)
            .map(|(_, name)| name.as_str())
    }
}

impl Drop for LockedNoteContent {
    fn drop(&mut self) {
        self.title.zeroize();
        self.body.zeroize();
        for tag in &mut self.tags {
            tag.zeroize();
        }
        for (_, name) in &mut self.attachment_names {
            name.zeroize();
        }
    }
}

impl fmt::Debug for LockedNoteContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockedNoteContent")
            .field("title", &"<redacted>")
            .field("body", &"<redacted>")
            .field("tag_count", &self.tags.len())
            .field("attachment_names", &self.attachment_names.len())
            .finish()
    }
}

pub(crate) fn kdf_params_valid(params: LockKdfParams) -> bool {
    (MIN_LOCK_KDF_MEMORY_KIB..=MAX_LOCK_KDF_MEMORY_KIB).contains(&params.memory_kib)
        && (1..=MAX_LOCK_KDF_ITERATIONS).contains(&params.iterations)
        && (1..=MAX_LOCK_KDF_PARALLELISM).contains(&params.parallelism)
        && params.memory_kib >= 8 * params.parallelism
}

pub(crate) fn hint_valid(hint: &str) -> bool {
    hint.len() <= MAX_LOCK_HINT_BYTES && !hint.chars().any(char::is_control)
}

fn derive(
    key_id: u32,
    password: &str,
    salt: &[u8; LOCK_SALT_BYTES],
    params: LockKdfParams,
) -> Result<LockKey, LockError> {
    if password.is_empty() {
        return Err(LockError::EmptyPassword);
    }
    if !kdf_params_valid(params) {
        return Err(LockError::InvalidParameters);
    }
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(LOCK_KEY_BYTES),
    )
    .map_err(|_| LockError::InvalidParameters)?;
    let mut bytes = Zeroizing::new([0_u8; LOCK_KEY_BYTES]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, bytes.as_mut_slice())
        .map_err(|_| LockError::InvalidParameters)?;
    Ok(LockKey { key_id, bytes })
}

fn verifier_aad(id: u32, salt: &[u8; LOCK_SALT_BYTES], params: LockKdfParams) -> Vec<u8> {
    let mut aad = Vec::with_capacity(VERIFIER_AAD.len() + 4 + LOCK_SALT_BYTES + 12);
    aad.extend_from_slice(VERIFIER_AAD);
    aad.extend_from_slice(&id.to_le_bytes());
    aad.extend_from_slice(salt);
    aad.extend_from_slice(&params.memory_kib.to_le_bytes());
    aad.extend_from_slice(&params.iterations.to_le_bytes());
    aad.extend_from_slice(&params.parallelism.to_le_bytes());
    aad
}

fn note_aad(key_id: u32, note_id: NoteId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(NOTE_AAD.len() + 12);
    aad.extend_from_slice(NOTE_AAD);
    aad.extend_from_slice(&key_id.to_le_bytes());
    aad.extend_from_slice(&note_id.get().to_le_bytes());
    aad
}

fn attachment_aad(key_id: u32, attachment_id: AttachmentId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ATTACHMENT_AAD.len() + 12);
    aad.extend_from_slice(ATTACHMENT_AAD);
    aad.extend_from_slice(&key_id.to_le_bytes());
    aad.extend_from_slice(&attachment_id.get().to_le_bytes());
    aad
}

/// Create a new password generation `id`: random salt, derived key and the
/// sealed verifier that later proves a password without storing a hash.
pub fn create_lock_key(
    id: u32,
    password: &str,
    hint: &str,
    params: LockKdfParams,
) -> Result<(LockKeyRecord, LockKey), LockError> {
    if id == 0 {
        return Err(LockError::InvalidParameters);
    }
    if !hint_valid(hint) {
        return Err(LockError::InvalidHint);
    }
    let mut salt = [0_u8; LOCK_SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|_| LockError::Random)?;
    let key = derive(id, password, &salt, params)?;
    let verifier = key.seal(&verifier_aad(id, &salt, params), VERIFIER_PLAINTEXT)?;
    Ok((
        LockKeyRecord {
            id,
            salt,
            kdf: params,
            verifier,
            hint: hint.to_owned(),
        },
        key,
    ))
}

/// Derive and verify the key for one stored password generation.
pub fn unlock_lock_key(record: &LockKeyRecord, password: &str) -> Result<LockKey, LockError> {
    let key = derive(record.id, password, &record.salt, record.kdf)?;
    let opened = key
        .open(
            &verifier_aad(record.id, &record.salt, record.kdf),
            &record.verifier,
        )
        .map_err(|_| LockError::WrongPassword)?;
    if opened.as_slice() != VERIFIER_PLAINTEXT {
        return Err(LockError::WrongPassword);
    }
    Ok(key)
}

/// Seal a note's content under `key`, bound to `note_id`.
pub fn seal_note(
    key: &LockKey,
    note_id: NoteId,
    content: &LockedNoteContent,
) -> Result<NoteLock, LockError> {
    let payload = encode_payload(content)?;
    let sealed = key.seal(&note_aad(key.key_id, note_id), &payload)?;
    if sealed.ciphertext.len() > MAX_SEALED_NOTE_BYTES {
        return Err(LockError::TooLarge);
    }
    Ok(NoteLock {
        key_id: key.key_id,
        sealed,
    })
}

/// Authenticate and decrypt one locked note's content.
pub fn open_note(
    key: &LockKey,
    note_id: NoteId,
    lock: &NoteLock,
) -> Result<LockedNoteContent, LockError> {
    if lock.key_id != key.key_id {
        return Err(LockError::WrongKey);
    }
    if lock.sealed.ciphertext.len() > MAX_SEALED_NOTE_BYTES {
        return Err(LockError::TooLarge);
    }
    let payload = key.open(&note_aad(key.key_id, note_id), &lock.sealed)?;
    decode_payload(&payload)
}

/// Seal one attachment's bytes as `nonce || ciphertext || tag`.
pub fn seal_attachment(
    key: &LockKey,
    attachment_id: AttachmentId,
    plaintext: &[u8],
) -> Result<Vec<u8>, LockError> {
    let sealed = key.seal(&attachment_aad(key.key_id, attachment_id), plaintext)?;
    let mut output = Vec::with_capacity(LOCK_NONCE_BYTES + sealed.ciphertext.len());
    output.extend_from_slice(&sealed.nonce);
    output.extend_from_slice(&sealed.ciphertext);
    Ok(output)
}

/// Authenticate and decrypt one sealed attachment file.
pub fn open_attachment(
    key: &LockKey,
    attachment_id: AttachmentId,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, LockError> {
    if sealed.len() < LOCK_NONCE_BYTES + LOCK_TAG_BYTES {
        return Err(LockError::Tampered);
    }
    let (nonce, ciphertext) = sealed.split_at(LOCK_NONCE_BYTES);
    let nonce: [u8; LOCK_NONCE_BYTES] = nonce.try_into().map_err(|_| LockError::Tampered)?;
    key.cipher()
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad: &attachment_aad(key.key_id, attachment_id),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| LockError::Tampered)
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), LockError> {
    let length = u32::try_from(bytes.len()).map_err(|_| LockError::TooLarge)?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn encode_payload(content: &LockedNoteContent) -> Result<Zeroizing<Vec<u8>>, LockError> {
    if content.tags.len() > MAX_TAGS_PER_NOTE
        || content.attachment_names.len() > MAX_ATTACHMENTS_PER_NOTE * 4
    {
        return Err(LockError::TooLarge);
    }
    let mut output = Zeroizing::new(Vec::with_capacity(
        content.title.len() + content.body.len() + 64,
    ));
    output.push(PAYLOAD_VERSION);
    put_bytes(&mut output, content.title.as_bytes())?;
    put_bytes(&mut output, content.body.as_bytes())?;
    output.extend_from_slice(&(content.tags.len() as u32).to_le_bytes());
    for tag in &content.tags {
        put_bytes(&mut output, tag.as_bytes())?;
    }
    output.extend_from_slice(&(content.attachment_names.len() as u32).to_le_bytes());
    for (id, name) in &content.attachment_names {
        output.extend_from_slice(&id.get().to_le_bytes());
        put_bytes(&mut output, name.as_bytes())?;
    }
    if output.len() > MAX_SEALED_NOTE_BYTES {
        return Err(LockError::TooLarge);
    }
    Ok(output)
}

struct PayloadReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> PayloadReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], LockError> {
        let end = self.cursor.checked_add(count).ok_or(LockError::Malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(LockError::Malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, LockError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| LockError::Malformed)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, LockError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| LockError::Malformed)?,
        ))
    }

    fn string(&mut self) -> Result<String, LockError> {
        let length = self.u32()? as usize;
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| LockError::Malformed)
    }
}

fn decode_payload(bytes: &[u8]) -> Result<LockedNoteContent, LockError> {
    let mut reader = PayloadReader { bytes, cursor: 0 };
    if reader.take(1)? != [PAYLOAD_VERSION] {
        return Err(LockError::Malformed);
    }
    let mut content = LockedNoteContent {
        title: reader.string()?,
        body: reader.string()?,
        tags: Vec::new(),
        attachment_names: Vec::new(),
    };
    let tag_count = reader.u32()? as usize;
    if tag_count > MAX_TAGS_PER_NOTE {
        return Err(LockError::Malformed);
    }
    for _ in 0..tag_count {
        content.tags.push(reader.string()?);
    }
    let name_count = reader.u32()? as usize;
    if name_count > MAX_ATTACHMENTS_PER_NOTE * 4 {
        return Err(LockError::Malformed);
    }
    for _ in 0..name_count {
        let id = AttachmentId::new(reader.u64()?).ok_or(LockError::Malformed)?;
        content.attachment_names.push((id, reader.string()?));
    }
    if reader.cursor != bytes.len() {
        return Err(LockError::Malformed);
    }
    Ok(content)
}

#[cfg(test)]
pub(crate) const TEST_LOCK_KDF: LockKdfParams = LockKdfParams {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> LockedNoteContent {
        LockedNoteContent {
            title: "Bank PIN".into(),
            body: "the secret body नमस्ते".into(),
            tags: vec!["private".into()],
            attachment_names: vec![(AttachmentId::new(3).unwrap(), "scan.png".into())],
        }
    }

    #[test]
    fn note_content_round_trips_and_binds_its_identity() {
        let (record, key) = create_lock_key(1, "hunter2", "pet", TEST_LOCK_KDF).unwrap();
        let note = NoteId::new(7).unwrap();
        let lock = seal_note(&key, note, &content()).unwrap();
        assert_eq!(lock.key_id, 1);
        let plain = "the secret body".as_bytes();
        assert!(!lock
            .sealed
            .ciphertext
            .windows(plain.len())
            .any(|window| window == plain));
        assert_eq!(open_note(&key, note, &lock).unwrap(), content());

        // Another note's identity, or the same bytes under a re-derived key
        // from the same password, must not authenticate.
        assert_eq!(
            open_note(&key, NoteId::new(8).unwrap(), &lock),
            Err(LockError::Tampered)
        );
        let again = unlock_lock_key(&record, "hunter2").unwrap();
        assert_eq!(open_note(&again, note, &lock).unwrap(), content());
    }

    #[test]
    fn wrong_password_is_rejected_by_the_verifier() {
        let (record, _) = create_lock_key(1, "correct horse", "", TEST_LOCK_KDF).unwrap();
        assert_eq!(
            unlock_lock_key(&record, "battery staple").unwrap_err(),
            LockError::WrongPassword
        );
        assert_eq!(
            unlock_lock_key(&record, "").unwrap_err(),
            LockError::EmptyPassword
        );
        assert!(unlock_lock_key(&record, "correct horse").is_ok());
    }

    #[test]
    fn any_flipped_bit_fails_authentication() {
        let (_, key) = create_lock_key(2, "pw", "", TEST_LOCK_KDF).unwrap();
        let note = NoteId::new(1).unwrap();
        let lock = seal_note(&key, note, &content()).unwrap();
        for index in [
            0,
            lock.sealed.ciphertext.len() / 2,
            lock.sealed.ciphertext.len() - 1,
        ] {
            let mut tampered = lock.clone();
            tampered.sealed.ciphertext[index] ^= 0x01;
            assert_eq!(open_note(&key, note, &tampered), Err(LockError::Tampered));
        }
        let mut nonce = lock.clone();
        nonce.sealed.nonce[0] ^= 0x80;
        assert_eq!(open_note(&key, note, &nonce), Err(LockError::Tampered));
        let mut key_id = lock;
        key_id.key_id = 3;
        assert_eq!(open_note(&key, note, &key_id), Err(LockError::WrongKey));
    }

    #[test]
    fn attachments_seal_and_detect_tampering() {
        let (_, key) = create_lock_key(1, "pw", "", TEST_LOCK_KDF).unwrap();
        let id = AttachmentId::new(9).unwrap();
        let image = b"\x89PNG\r\n\x1a\nsecret pixels".to_vec();
        let sealed = seal_attachment(&key, id, &image).unwrap();
        assert_eq!(
            sealed.len() as u64,
            image.len() as u64 + SEALED_ATTACHMENT_OVERHEAD
        );
        assert!(!sealed.windows(6).any(|window| window == b"secret"));
        assert_eq!(
            open_attachment(&key, id, &sealed).unwrap().as_slice(),
            image
        );
        assert_eq!(
            open_attachment(&key, AttachmentId::new(10).unwrap(), &sealed).unwrap_err(),
            LockError::Tampered
        );
        let mut tampered = sealed;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert_eq!(
            open_attachment(&key, id, &tampered).unwrap_err(),
            LockError::Tampered
        );
    }

    #[test]
    fn hostile_parameters_are_refused_before_deriving() {
        let mut params = DEFAULT_LOCK_KDF;
        params.memory_kib = MAX_LOCK_KDF_MEMORY_KIB + 1;
        assert_eq!(
            create_lock_key(1, "pw", "", params).unwrap_err(),
            LockError::InvalidParameters
        );
        assert_eq!(
            create_lock_key(1, "pw", "a\nb", TEST_LOCK_KDF).unwrap_err(),
            LockError::InvalidHint
        );
        assert!(kdf_params_valid(DEFAULT_LOCK_KDF));
    }

    #[test]
    fn default_parameters_derive_within_the_low_spec_budget() {
        // The real cost: one derivation with the shipped parameters. The
        // 250 ms budget is for the four-thread reference laptop; CI runners
        // are comparable, and debug builds are far slower, so only release
        // test runs enforce the bound.
        let started = std::time::Instant::now();
        let (record, _) =
            create_lock_key(1, "a reasonable password", "", DEFAULT_LOCK_KDF).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(record.kdf, DEFAULT_LOCK_KDF);
        eprintln!("Argon2id with the shipped parameters took {elapsed:?}");
        if !cfg!(debug_assertions) {
            assert!(elapsed.as_millis() <= 250, "Argon2id took {elapsed:?}");
        }
    }
}
