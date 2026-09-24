//! Plain-text editing on a string and a selection, with no UI toolkit: the
//! model behind [`crate::text_field::TextField`]. Offsets are UTF-8 byte
//! offsets; caret movement steps over whole grapheme clusters and words the
//! way an AppKit text field does.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Where a caret movement goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    /// ←: one character back.
    Left,
    /// →: one character forward.
    Right,
    /// ⌥←: to the start of the word before the caret.
    WordLeft,
    /// ⌥→: to the end of the word after the caret.
    WordRight,
    /// ⌘← or Home: to the start of the text.
    Start,
    /// ⌘→ or End: to the end of the text.
    End,
}

/// Text plus a selection whose head (the moving end) is `end` unless the
/// selection is reversed, and an optional marked (IME composition) range.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextEdit {
    text: String,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
}

impl TextEdit {
    pub fn new(text: impl Into<String>, selection: Range<usize>) -> Self {
        let mut edit = Self {
            text: text.into(),
            ..Self::default()
        };
        edit.select(selection);
        edit
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// The caret: the end of the selection that moves.
    pub fn head(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn anchor(&self) -> usize {
        if self.reversed {
            self.selection.end
        } else {
            self.selection.start
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    /// Clamps to the text and rounds down to a character boundary.
    pub fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        start.min(end)..start.max(end)
    }

    /// Selects `range` with the caret at its end.
    pub fn select(&mut self, range: Range<usize>) {
        self.selection = self.clamp_range(range);
        self.reversed = false;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    pub fn set_cursor(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selection = offset..offset;
        self.reversed = false;
    }

    /// Moves the caret to `offset`, keeping the anchor (shift-click, drag).
    pub fn select_to(&mut self, offset: usize) {
        let head = self.clamp(offset);
        let anchor = self.anchor();
        if head < anchor {
            self.selection = head..anchor;
            self.reversed = true;
        } else {
            self.selection = anchor..head;
            self.reversed = false;
        }
    }

    /// Selects the word (or the run of spaces or punctuation) around
    /// `offset`, as a double-click does.
    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        let runs = runs(&self.text);
        let range = runs
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .or(runs.last())
            .map(|(range, _)| range.clone())
            .unwrap_or(offset..offset);
        self.select(range);
    }

    /// Moves the caret. Without `extend`, a plain ← or → on a selection
    /// collapses it to that side, as AppKit does.
    pub fn move_by(&mut self, motion: Motion, extend: bool) {
        if extend {
            let target = self.target(self.head(), motion);
            self.select_to(target);
            return;
        }
        let collapsed = self.selection.is_empty();
        let target = match motion {
            Motion::Left if !collapsed => self.selection.start,
            Motion::Right if !collapsed => self.selection.end,
            Motion::Left | Motion::WordLeft | Motion::Start => {
                self.target(self.selection.start, motion)
            }
            Motion::Right | Motion::WordRight | Motion::End => {
                self.target(self.selection.end, motion)
            }
        };
        self.set_cursor(target);
    }

    fn target(&self, from: usize, motion: Motion) -> usize {
        match motion {
            Motion::Left => previous_grapheme(&self.text, from),
            Motion::Right => next_grapheme(&self.text, from),
            Motion::WordLeft => word_start_before(&self.text, from),
            Motion::WordRight => word_end_after(&self.text, from),
            Motion::Start => 0,
            Motion::End => self.text.len(),
        }
    }

    /// Replaces `range` (or the marked text, or else the selection) with
    /// `new_text`, leaving the caret after it and ending any composition.
    pub fn replace(&mut self, range: Option<Range<usize>>, new_text: &str) {
        let range = self.edit_range(range);
        self.text.replace_range(range.clone(), new_text);
        self.set_cursor(range.start + new_text.len());
        self.marked = None;
    }

    /// Replaces like [`Self::replace`] but marks the new text as an IME
    /// composition; `selected` is a range inside `new_text`.
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = self.edit_range(range);
        self.text.replace_range(range.clone(), new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        match selected {
            Some(selected) => self.select(range.start + selected.start..range.start + selected.end),
            None => self.set_cursor(range.start + new_text.len()),
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    fn edit_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .map(|range| self.clamp_range(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone())
    }

    /// Backspace (`Left`, `WordLeft`, `Start`) and forward delete (`Right`,
    /// `WordRight`, `End`): removes the selection if there is one, else from
    /// the caret to the motion's target. Returns false when nothing changed.
    pub fn delete(&mut self, motion: Motion) -> bool {
        if !self.selection.is_empty() {
            self.replace(Some(self.selection.clone()), "");
            return true;
        }
        let head = self.head();
        let target = self.target(head, motion);
        if target == head {
            return false;
        }
        self.replace(Some(target.min(head)..target.max(head)), "");
        true
    }

    /// Removes and returns the selection (⌘X).
    pub fn cut(&mut self) -> Option<String> {
        if self.selection.is_empty() {
            return None;
        }
        let taken = self.selected_text().to_owned();
        self.replace(Some(self.selection.clone()), "");
        Some(taken)
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        utf8_to_utf16(&self.text, offset)
    }

    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        utf16_to_utf8(&self.text, offset)
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

/// A UTF-8 offset in `text` as a UTF-16 offset.
pub fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    text.char_indices()
        .take_while(|(index, _)| *index < offset)
        .map(|(_, ch)| ch.len_utf16())
        .sum()
}

/// A UTF-16 offset in `text` as a UTF-8 offset (rounded forward to a
/// character boundary).
pub fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf16 = 0;
    for (index, ch) in text.char_indices() {
        if utf16 >= offset {
            return index;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

/// Text pasted or typed into a one-paragraph field: line breaks and tabs
/// become spaces and other control characters are dropped.
pub fn single_paragraph(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => Some(' '),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect()
}

fn previous_grapheme(text: &str, from: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .map(|(index, _)| index)
        .find(|index| *index < from)
        .unwrap_or(0)
}

fn next_grapheme(text: &str, from: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index > from)
        .unwrap_or(text.len())
}

/// The text as alternating runs of word characters (letters, digits and
/// their combining marks) and everything else. Unlike UAX #29 word
/// boundaries, a dot always separates words, so ⌥→ stops before a file
/// name's extension.
fn runs(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut runs: Vec<(Range<usize>, bool)> = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        let word = grapheme.chars().next().is_some_and(char::is_alphanumeric);
        let end = start + grapheme.len();
        match runs.last_mut() {
            Some((range, kind)) if *kind == word => range.end = end,
            _ => runs.push((start..end, word)),
        }
    }
    runs
}

fn word_start_before(text: &str, from: usize) -> usize {
    runs(text)
        .into_iter()
        .filter(|(range, word)| *word && range.start < from)
        .map(|(range, _)| range.start)
        .next_back()
        .unwrap_or(0)
}

fn word_end_after(text: &str, from: usize) -> usize {
    runs(text)
        .into_iter()
        .find(|(range, word)| *word && range.end > from)
        .map(|(range, _)| range.end)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_selection() {
        let mut edit = TextEdit::new("Budget 2026.xlsx", 0..11);
        edit.replace(None, "Plan");
        assert_eq!(edit.text(), "Plan.xlsx");
        assert_eq!(edit.selection(), 4..4);
    }

    #[test]
    fn arrows_collapse_then_move_by_grapheme() {
        let mut edit = TextEdit::new("ae\u{301}b", 1..4);
        edit.move_by(Motion::Left, false);
        assert_eq!(edit.selection(), 1..1);
        edit.move_by(Motion::Right, false);
        // "e" plus the combining acute is one character.
        assert_eq!(edit.selection(), 4..4);
        edit.move_by(Motion::Left, false);
        assert_eq!(edit.selection(), 1..1);
        let mut edit = TextEdit::new("ab", 0..2);
        edit.move_by(Motion::Right, false);
        assert_eq!(edit.selection(), 2..2);
    }

    #[test]
    fn option_arrows_move_by_word() {
        let mut edit = TextEdit::new("Budget 2026.xlsx", 0..0);
        edit.move_by(Motion::WordRight, false);
        assert_eq!(edit.head(), 6);
        edit.move_by(Motion::WordRight, false);
        assert_eq!(edit.head(), 11);
        edit.move_by(Motion::WordRight, false);
        assert_eq!(edit.head(), 16);
        edit.move_by(Motion::WordRight, false);
        assert_eq!(edit.head(), 16);
        edit.move_by(Motion::WordLeft, false);
        assert_eq!(edit.head(), 12);
        edit.set_cursor(9);
        edit.move_by(Motion::WordLeft, false);
        assert_eq!(edit.head(), 7);
        edit.move_by(Motion::WordLeft, false);
        assert_eq!(edit.head(), 0);
    }

    #[test]
    fn shift_extends_from_the_anchor_in_either_direction() {
        let mut edit = TextEdit::new("hello world", 5..5);
        edit.move_by(Motion::Left, true);
        edit.move_by(Motion::Left, true);
        assert_eq!(edit.selection(), 3..5);
        assert!(edit.is_reversed());
        edit.move_by(Motion::End, true);
        assert_eq!(edit.selection(), 5..11);
        assert!(!edit.is_reversed());
        edit.move_by(Motion::Start, false);
        assert_eq!(edit.selection(), 0..0);
        edit.select_to(3);
        edit.select_to(1);
        assert_eq!(edit.selection(), 0..1);
    }

    #[test]
    fn deleting_backward_and_forward() {
        let mut edit = TextEdit::new("one two", 7..7);
        assert!(edit.delete(Motion::Left));
        assert_eq!(edit.text(), "one tw");
        assert!(edit.delete(Motion::WordLeft));
        assert_eq!(edit.text(), "one ");
        edit.set_cursor(0);
        assert!(!edit.delete(Motion::Left));
        assert!(edit.delete(Motion::Right));
        assert_eq!(edit.text(), "ne ");
        edit.select_all();
        assert!(edit.delete(Motion::Left));
        assert_eq!(edit.text(), "");
        let mut edit = TextEdit::new("abc def", 3..3);
        assert!(edit.delete(Motion::Start));
        assert_eq!(edit.text(), " def");
    }

    #[test]
    fn cut_and_select_all() {
        let mut edit = TextEdit::new("notes.txt", 0..5);
        assert_eq!(edit.cut().as_deref(), Some("notes"));
        assert_eq!(edit.text(), ".txt");
        assert_eq!(edit.cut(), None);
        edit.select_all();
        assert_eq!(edit.selected_text(), ".txt");
    }

    #[test]
    fn composition_marks_and_replaces_its_text() {
        let mut edit = TextEdit::new("ab", 1..1);
        edit.replace_and_mark(None, "´", Some(1..1 + '´'.len_utf8()));
        assert_eq!(edit.text(), "a´b");
        assert_eq!(edit.marked(), Some(1..1 + '´'.len_utf8()));
        edit.replace(None, "é");
        assert_eq!(edit.text(), "aéb");
        assert_eq!(edit.marked(), None);
        assert_eq!(edit.head(), 1 + 'é'.len_utf8());
    }

    #[test]
    fn double_click_selects_a_word() {
        let mut edit = TextEdit::new("Lease agreement.pdf", 0..0);
        edit.select_word_at(8);
        assert_eq!(edit.selected_text(), "agreement");
        edit.select_word_at(19);
        assert_eq!(edit.selected_text(), "pdf");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let edit = TextEdit::new("a😀b", 0..0);
        assert_eq!(edit.offset_to_utf16(5), 3);
        assert_eq!(edit.offset_from_utf16(3), 5);
        assert_eq!(edit.offset_from_utf16(9), 6);
        assert_eq!(edit.range_to_utf16(&(1..5)), 1..3);
    }

    #[test]
    fn offsets_are_clamped_to_boundaries() {
        let edit = TextEdit::new("é", 1..9);
        assert_eq!(edit.selection(), 0..2);
    }

    #[test]
    fn pasted_text_stays_on_one_paragraph() {
        assert_eq!(single_paragraph("a\nb\tc\u{7}"), "a b c");
    }
}
