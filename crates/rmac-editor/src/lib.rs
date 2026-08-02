//! `rmac-editor` — the shared text-editing core for rmac apps.
//!
//! Wraps gpui-component's rope-backed `InputState` with rmac's standard
//! configuration so the Text Editor and Notes (and anything else with a text
//! body) behave and look identical. Re-exports the `Input` element for rendering.

use std::ops::Range;

use gpui::{AppContext as _, Context, Entity, EntityInputHandler, UTF16Selection, Window};

pub use gpui_component::input::{Input, InputState};

/// Matches the largest plain-text document accepted by Text Editor.
pub const MAX_ACCESSIBLE_EDITOR_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorAccessibilitySnapshot {
    /// Exact editor text. Offsets below count Unicode scalar values, not UTF-8
    /// bytes or UTF-16 code units.
    pub text: String,
    /// Active selection head, available only while input is enabled.
    pub caret: Option<usize>,
    /// Non-empty selection as a half-open Unicode-scalar range.
    pub selection: Option<Range<usize>>,
    /// Active IME composition as a half-open Unicode-scalar range.
    pub marked: Option<Range<usize>>,
    pub character_count: usize,
    pub line_count: usize,
    pub editable: bool,
    pub input_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityProjectionError {
    TextLimit,
    InvalidCaret,
    InvalidSelection,
    InvalidMarkedRange,
}

/// Create a standard multi-line, soft-wrapped editor state.
///
/// Render it with [`Input::new`]`(&state).h_full().appearance(false)`.
pub fn multiline<T: 'static>(
    placeholder: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| {
        InputState::new(window, cx)
            .multi_line(true)
            .soft_wrap(true)
            .placeholder(placeholder)
    })
}

/// Read the current text out of an editor state.
pub fn value<T: 'static>(state: &Entity<InputState>, cx: &Context<T>) -> String {
    state.read(cx).value().to_string()
}

/// Read the same UTF-16 selection and marked-text state exposed to the
/// platform input method, then project it for a future accessibility-tree
/// adapter. This does not itself publish an accessibility node.
pub fn accessibility_snapshot(
    state: &mut InputState,
    editable: bool,
    input_enabled: bool,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> Result<EditorAccessibilitySnapshot, AccessibilityProjectionError> {
    let caret_utf8 = state.cursor();
    let selection = state.selected_text_range(true, window, cx);
    let marked = state.marked_text_range(window, cx);
    project_accessible_text(
        &state.value(),
        selection,
        marked,
        Some(caret_utf8),
        editable,
        input_enabled,
    )
}

/// Project authoritative editor text and GPUI's UTF-16 input ranges into one
/// bounded Unicode-scalar snapshot. `caret_utf8` is the exact selection head
/// reported by the input state; it must not be inferred from range direction.
pub fn project_accessible_text(
    text: &str,
    selection_utf16: Option<UTF16Selection>,
    marked_utf16: Option<Range<usize>>,
    caret_utf8: Option<usize>,
    editable: bool,
    input_enabled: bool,
) -> Result<EditorAccessibilitySnapshot, AccessibilityProjectionError> {
    if text.len() > MAX_ACCESSIBLE_EDITOR_BYTES {
        return Err(AccessibilityProjectionError::TextLimit);
    }

    let selection = selection_utf16
        .map(|selection| {
            scalar_range(text, selection.range)
                .ok_or(AccessibilityProjectionError::InvalidSelection)
        })
        .transpose()?;
    let caret = caret_utf8
        .map(|offset| {
            scalar_byte_offset(text, offset).ok_or(AccessibilityProjectionError::InvalidCaret)
        })
        .transpose()?;
    let marked = marked_utf16
        .map(|range| {
            scalar_range(text, range).ok_or(AccessibilityProjectionError::InvalidMarkedRange)
        })
        .transpose()?
        .filter(|range| !range.is_empty());
    let character_count = text.chars().count();

    Ok(EditorAccessibilitySnapshot {
        text: text.to_string(),
        caret: input_enabled.then_some(caret).flatten(),
        selection: selection.filter(|range| !range.is_empty()),
        marked,
        character_count,
        line_count: text.chars().filter(|character| *character == '\n').count() + 1,
        editable,
        input_enabled,
    })
}

fn scalar_range(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end {
        return None;
    }
    let start = scalar_offset(text, range.start)?;
    let end = scalar_offset(text, range.end)?;
    Some(start..end)
}

fn scalar_offset(text: &str, target_utf16: usize) -> Option<usize> {
    let mut utf16_offset = 0;
    let mut scalar_offset = 0;
    for character in text.chars() {
        if utf16_offset == target_utf16 {
            return Some(scalar_offset);
        }
        utf16_offset = utf16_offset.checked_add(character.len_utf16())?;
        scalar_offset = scalar_offset.checked_add(1)?;
        if utf16_offset > target_utf16 {
            return None;
        }
    }
    (utf16_offset == target_utf16).then_some(scalar_offset)
}

fn scalar_byte_offset(text: &str, target_utf8: usize) -> Option<usize> {
    (target_utf8 <= text.len() && text.is_char_boundary(target_utf8))
        .then(|| text[..target_utf8].chars().count())
}

/// Derive a short title from note/body text: first non-empty line, trimmed.
pub fn title_from_body(body: &str, fallback: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let t: String = l.trim_start_matches('#').trim().chars().take(60).collect();
            if t.is_empty() {
                fallback.to_string()
            } else {
                t
            }
        })
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessible_text_uses_scalar_offsets_for_selection_caret_and_ime() {
        let snapshot = project_accessible_text(
            "A😀e\u{301}\nZ",
            Some(UTF16Selection {
                range: 1..5,
                reversed: false,
            }),
            Some(3..5),
            Some(8),
            true,
            true,
        )
        .unwrap();

        assert_eq!(snapshot.text, "A😀e\u{301}\nZ");
        assert_eq!(snapshot.selection, Some(1..4));
        assert_eq!(snapshot.caret, Some(4));
        assert_eq!(snapshot.marked, Some(2..4));
        assert_eq!((snapshot.character_count, snapshot.line_count), (6, 2));
        assert!(snapshot.editable);
        assert!(snapshot.input_enabled);
    }

    #[test]
    fn projection_preserves_reverse_selection_and_rejects_split_surrogates() {
        let reversed = project_accessible_text(
            "A😀B",
            Some(UTF16Selection {
                range: 1..4,
                reversed: true,
            }),
            None,
            Some(1),
            true,
            true,
        )
        .unwrap();
        assert_eq!(reversed.selection, Some(1..3));
        assert_eq!(reversed.caret, Some(1));

        let disabled = project_accessible_text(
            "A😀B",
            Some(UTF16Selection {
                range: 1..4,
                reversed: false,
            }),
            None,
            Some(5),
            false,
            false,
        )
        .unwrap();
        assert_eq!(disabled.selection, Some(1..3));
        assert_eq!(disabled.caret, None);

        assert_eq!(
            project_accessible_text(
                "A😀B",
                Some(UTF16Selection {
                    range: 2..3,
                    reversed: false,
                }),
                None,
                Some(1),
                true,
                true,
            ),
            Err(AccessibilityProjectionError::InvalidSelection)
        );
        assert_eq!(
            project_accessible_text("A😀B", None, None, Some(2), true, true),
            Err(AccessibilityProjectionError::InvalidCaret)
        );
    }
}
