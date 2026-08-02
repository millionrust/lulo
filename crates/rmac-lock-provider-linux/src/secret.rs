use std::fmt;

use zeroize::Zeroize as _;

/// Linux-PAM currently bounds a conversation response to 512 bytes. Keeping
/// the allocation fixed also prevents secret copies caused by `Vec` growth.
pub const MAX_SECRET_BYTES: usize = 512;

pub struct SecretInput {
    bytes: Vec<u8>,
}

impl Default for SecretInput {
    fn default() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_SECRET_BYTES),
        }
    }
}

impl SecretInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn character_count(&self) -> usize {
        // Construction preserves UTF-8, so failure would mean memory
        // corruption rather than user input and is treated as empty.
        std::str::from_utf8(&self.bytes)
            .map(|value| value.chars().count())
            .unwrap_or(0)
    }

    pub fn push(&mut self, character: char) -> Result<(), Error> {
        let mut encoded = [0_u8; 4];
        let value = character.encode_utf8(&mut encoded).as_bytes();
        let result = if self.bytes.len() + value.len() > MAX_SECRET_BYTES {
            Err(Error::Full)
        } else {
            self.bytes.extend_from_slice(value);
            Ok(())
        };
        encoded.zeroize();
        result
    }

    /// Append one already-decoded UTF-8 fragment atomically. If it does not
    /// fit, no prefix is retained.
    pub fn push_text(&mut self, value: &str) -> Result<(), Error> {
        if self.bytes.len() + value.len() > MAX_SECRET_BYTES {
            return Err(Error::Full);
        }
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub fn backspace(&mut self) -> bool {
        let Some(index) = std::str::from_utf8(&self.bytes)
            .ok()
            .and_then(|value| value.char_indices().next_back().map(|(index, _)| index))
        else {
            return false;
        };
        self.bytes[index..].zeroize();
        self.bytes.truncate(index);
        true
    }

    pub fn clear(&mut self) {
        self.bytes.zeroize();
    }

    /// Move the allocation to the bounded PAM worker without copying it.
    pub fn finish(mut self) -> SecretResponse {
        SecretResponse {
            bytes: std::mem::take(&mut self.bytes),
        }
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretInput(<redacted>)")
    }
}

impl Drop for SecretInput {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

pub struct SecretResponse {
    bytes: Vec<u8>,
}

impl SecretResponse {
    /// Scope plaintext access to the PAM conversation callback. Callers must
    /// not clone, log, cache, or return a borrow from this closure.
    pub fn expose<R>(&self, use_secret: impl FnOnce(&[u8]) -> R) -> R {
        use_secret(&self.bytes)
    }
}

impl fmt::Debug for SecretResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretResponse(<redacted>)")
    }
}

impl Drop for SecretResponse {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Full,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authentication response is too long")
    }
}

impl std::error::Error for Error {}
