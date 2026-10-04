//! The password as typed: a fixed-capacity buffer that is zeroized on every
//! edit and on drop, never formatted, never cloned. It only ever leaves the
//! process as the bytes written to polkit's helper.

use std::fmt;

use zeroize::Zeroize as _;

/// Linux-PAM bounds a conversation response to 512 bytes. A fixed
/// allocation also means `Vec` growth never leaves a stale copy behind.
pub const MAX_SECRET_BYTES: usize = 512;

pub struct Secret {
    bytes: Vec<u8>,
}

impl Default for Secret {
    fn default() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_SECRET_BYTES),
        }
    }
}

impl Secret {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// How many bullets to draw.
    pub fn character_count(&self) -> usize {
        std::str::from_utf8(&self.bytes)
            .map(|value| value.chars().count())
            .unwrap_or(0)
    }

    /// Append typed text. Control characters (a newline would end the
    /// helper's line early) are dropped, and text that does not fit is
    /// ignored as a whole: returns false then.
    pub fn push_str(&mut self, text: &str) -> bool {
        let mut accepted = true;
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            let mut encoded = [0_u8; 4];
            let value = character.encode_utf8(&mut encoded).as_bytes();
            if self.bytes.len() + value.len() > MAX_SECRET_BYTES {
                accepted = false;
            } else {
                self.bytes.extend_from_slice(value);
            }
            encoded.zeroize();
        }
        accepted
    }

    /// Delete the last character.
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

    /// Move the bytes out without copying them, leaving this field empty.
    pub fn take(&mut self) -> Secret {
        Secret {
            bytes: std::mem::replace(&mut self.bytes, Vec::with_capacity(MAX_SECRET_BYTES)),
        }
    }

    /// Scoped plaintext access for the helper write. Callers must not copy,
    /// log or keep the slice.
    pub(crate) fn expose<R>(&self, use_secret: impl FnOnce(&[u8]) -> R) -> R {
        use_secret(&self.bytes)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_count_characters_and_never_print_the_value() {
        let mut secret = Secret::new();
        assert!(secret.push_str("pä\nss"));
        assert_eq!(secret.character_count(), 4);
        assert!(secret.backspace());
        assert_eq!(secret.character_count(), 3);
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        secret.expose(|bytes| assert_eq!(bytes, "päs".as_bytes()));
        let moved = secret.take();
        assert!(secret.is_empty());
        moved.expose(|bytes| assert_eq!(bytes, "päs".as_bytes()));
    }

    #[test]
    fn a_full_field_ignores_more_input() {
        let mut secret = Secret::new();
        assert!(secret.push_str(&"a".repeat(MAX_SECRET_BYTES)));
        assert!(!secret.push_str("b"));
        assert_eq!(secret.character_count(), MAX_SECRET_BYTES);
        secret.clear();
        assert!(secret.is_empty());
        assert!(!secret.backspace());
    }
}
