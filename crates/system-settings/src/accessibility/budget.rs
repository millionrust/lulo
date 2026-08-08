//! Bounded semantic-text accounting.

use super::model::*;

#[derive(Default)]
pub(super) struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    pub(super) fn add_required(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.add_optional(text, max, multiline)
    }

    pub(super) fn add_optional(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.len() > max {
            return Err(AccessibilityProjectionError::TextValueLimit);
        }
        if text.chars().any(|character| {
            character.is_control() && !(multiline && matches!(character, '\n' | '\t'))
        }) {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > MAX_SEMANTIC_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}
