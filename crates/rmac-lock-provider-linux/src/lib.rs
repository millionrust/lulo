//! Linux lock-provider adapter primitives.
//!
//! The Wayland and PAM adapters remain intentionally absent until their
//! dependency review gates pass. This crate currently owns only bounded secret
//! input: one fixed-capacity allocation, no cloning, redacted diagnostics, and
//! guaranteed zeroization on backspace, clear, transfer, and drop.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_input_and_backspace_preserve_scalar_boundaries() {
        let mut input = SecretInput::new();
        input.push('a').unwrap();
        input.push('é').unwrap();
        input.push('🔒').unwrap();
        assert_eq!(input.character_count(), 3);
        assert!(input.backspace());
        assert_eq!(input.character_count(), 2);
        let response = input.finish();
        response.expose(|bytes| assert_eq!(bytes, "aé".as_bytes()));
    }

    #[test]
    fn fixed_capacity_rejects_overflow_without_partial_input() {
        let mut input = SecretInput::new();
        for _ in 0..MAX_SECRET_BYTES {
            input.push('x').unwrap();
        }
        assert_eq!(input.character_count(), MAX_SECRET_BYTES);
        assert_eq!(input.push('y'), Err(Error::Full));
        assert_eq!(input.character_count(), MAX_SECRET_BYTES);
    }

    #[test]
    fn clear_erases_content_and_reuses_the_bounded_allocation() {
        let mut input = SecretInput::new();
        input.push('s').unwrap();
        input.push('e').unwrap();
        input.clear();
        assert!(input.is_empty());
        assert!(!input.backspace());
        input.push('n').unwrap();
        input.finish().expose(|bytes| assert_eq!(bytes, b"n"));
    }

    #[test]
    fn diagnostics_never_include_secret_or_length() {
        let mut input = SecretInput::new();
        for character in "private password".chars() {
            input.push(character).unwrap();
        }
        let input_debug = format!("{input:?}");
        let response = input.finish();
        let response_debug = format!("{response:?}");
        for debug in [input_debug, response_debug] {
            assert!(!debug.contains("private password"));
            assert!(!debug.contains("16"));
            assert!(debug.contains("<redacted>"));
        }
    }
}
