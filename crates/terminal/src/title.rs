//! Private, bounded presentation state for one terminal session title.

use std::sync::{Arc, Mutex};

pub(super) const MAX_TITLE_BYTES: usize = 256;

#[derive(Clone, Default)]
pub(super) struct SessionTitle {
    value: Arc<Mutex<Option<String>>>,
}

impl SessionTitle {
    /// Replace the visible title and report whether presentation changed.
    pub(super) fn set(&self, value: Option<&str>) -> bool {
        let normalized = value.and_then(normalize);
        let Ok(mut current) = self.value.lock() else {
            return false;
        };
        if *current == normalized {
            return false;
        }
        *current = normalized;
        true
    }

    pub(super) fn current(&self) -> Option<String> {
        self.value.lock().ok()?.clone()
    }
}

fn normalize(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len().min(MAX_TITLE_BYTES));
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if character.is_control() || is_directional_control(character) {
            continue;
        }
        if pending_space {
            if output.len() + 1 > MAX_TITLE_BYTES {
                break;
            }
            output.push(' ');
            pending_space = false;
        }
        if output.len() + character.len_utf8() > MAX_TITLE_BYTES {
            break;
        }
        output.push(character);
    }
    (!output.is_empty()).then_some(output)
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_private_safe_normalized_and_utf8_bounded() {
        let title = SessionTitle::default();
        assert!(title.set(Some("  vim\t—\nnotes\u{202e}.txt  ")));
        assert_eq!(title.current().as_deref(), Some("vim — notes.txt"));
        assert!(!title.set(Some("vim — notes.txt")));

        let oversized = format!("{}😀", "a".repeat(MAX_TITLE_BYTES - 1));
        assert!(title.set(Some(&oversized)));
        let bounded = title.current().unwrap();
        assert_eq!(bounded.len(), MAX_TITLE_BYTES - 1);
        assert!(bounded.is_char_boundary(bounded.len()));

        assert!(title.set(Some("\n\u{202e}\t")));
        assert_eq!(title.current(), None);
        assert!(!title.set(None));
    }

    #[test]
    fn clones_share_only_their_exact_session_title() {
        let first = SessionTitle::default();
        let first_proxy = first.clone();
        let second = SessionTitle::default();
        assert!(first_proxy.set(Some("first")));
        assert_eq!(first.current().as_deref(), Some("first"));
        assert_eq!(second.current(), None);
    }
}
