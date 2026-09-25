//! Format ▸ Bold/Italic/Paragraph Style/Lists: Markdown-marker commands over
//! the note body. Notes keeps Markdown as its storage format (see
//! `rmac-notes-storage/src/markdown_preview.rs`), so these commands edit the
//! same Markdown source the body field already holds — a paragraph style is
//! a line-prefix marker, and Bold/Italic wrap the current selection. This is
//! the "Markdown-marker shortcuts" half of docs/parity.md's NOTES-01/NOTES-03
//! sizing, not full WYSIWYG editing: there is no live-styled canvas, so the
//! marker characters stay visible until Markdown Preview (or a future rich
//! editor) renders them.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ParagraphStyle {
    Title,
    Heading,
    Subheading,
    Body,
    Monospaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListMarker {
    Bulleted,
    Numbered,
}

/// The current line's text with one leading Markdown marker (heading,
/// checklist, list, or a whole-line code span) removed, so switching
/// paragraph styles replaces rather than stacks markers.
fn strip_leading_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix('#') {
        let extra_hashes = rest.chars().take_while(|&c| c == '#').count();
        let hashes = 1 + extra_hashes;
        if hashes <= 6 {
            if let Some(stripped) = line[hashes..].strip_prefix(' ') {
                return stripped;
            }
        }
    }
    for prefix in ["- [ ] ", "- [x] ", "- [X] ", "- ", "* "] {
        if let Some(stripped) = line.strip_prefix(prefix) {
            return stripped;
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        if let Some(stripped) = line[digits..].strip_prefix(". ") {
            return stripped;
        }
    }
    if line.len() >= 2 && line.starts_with('`') && line.ends_with('`') {
        return &line[1..line.len() - 1];
    }
    line
}

/// The byte range of the line in `value` that contains `cursor`.
fn current_line_range(value: &str, cursor: usize) -> std::ops::Range<usize> {
    let cursor = cursor.min(value.len());
    let start = value[..cursor]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = value[cursor..]
        .find('\n')
        .map(|index| cursor + index)
        .unwrap_or(value.len());
    start..end
}

impl NotesView {
    /// Whether the body accepts an edit right now — mirrors
    /// `edit_recovery_controller::assistive_fields_editable` and
    /// `insert_checklist`'s own guard.
    fn body_format_editable(&self) -> bool {
        self.is_interactive_ready()
            && !self.markdown_preview_visible
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted)
    }

    fn apply_to_current_line(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&str) -> String,
    ) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let value = state.value().to_string();
            let cursor = state.cursor();
            let range = current_line_range(&value, cursor);
            let Some(line) = value.get(range.clone()) else {
                return;
            };
            let new_line = build(line);
            state.set_selected_range(range, cx);
            state.replace(new_line, window, cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
        cx.notify();
    }

    /// Format ▸ Paragraph Style: Title/Heading/Subheading are Markdown
    /// headings, Body clears any marker, and Monospaced wraps the whole
    /// line as an inline code span (the preview already draws `code` runs
    /// in the monospace font).
    pub(super) fn set_paragraph_style(
        &mut self,
        style: ParagraphStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_current_line(window, cx, |line| {
            let body = strip_leading_marker(line);
            match style {
                ParagraphStyle::Title => format!("# {body}"),
                ParagraphStyle::Heading => format!("## {body}"),
                ParagraphStyle::Subheading => format!("### {body}"),
                ParagraphStyle::Body => body.to_string(),
                ParagraphStyle::Monospaced if body.is_empty() => String::new(),
                ParagraphStyle::Monospaced => format!("`{body}`"),
            }
        });
    }

    /// Format ▸ Lists: Bulleted and Numbered are line-prefix Markdown
    /// markers, like Checklist (⇧⌘L) already inserts.
    pub(super) fn insert_list_marker(
        &mut self,
        marker: ListMarker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_current_line(window, cx, |line| {
            let body = strip_leading_marker(line);
            match marker {
                ListMarker::Bulleted => format!("- {body}"),
                ListMarker::Numbered => format!("1. {body}"),
            }
        });
    }

    /// Wrap (or unwrap) the current selection in `prefix`/`suffix`, the
    /// Markdown-insertion form of Bold (`**`) and Italic (`_`). An empty
    /// selection gets an empty pair with the caret left inside it.
    fn apply_inline_markdown(
        &mut self,
        prefix: &'static str,
        suffix: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let range = state.selected_range();
            let value = state.value().to_string();
            let selected = value.get(range).unwrap_or_default().to_string();
            let already_wrapped = selected.len() >= prefix.len() + suffix.len()
                && selected.starts_with(prefix)
                && selected.ends_with(suffix);
            if already_wrapped {
                let inner = selected[prefix.len()..selected.len() - suffix.len()].to_string();
                state.replace(inner, window, cx);
            } else if selected.is_empty() {
                state.replace(format!("{prefix}{suffix}"), window, cx);
                let caret = state.cursor().saturating_sub(suffix.len());
                state.set_selected_range(caret..caret, cx);
            } else {
                state.replace(format!("{prefix}{selected}{suffix}"), window, cx);
            }
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
        cx.notify();
    }

    pub(super) fn toggle_bold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("**", "**", window, cx);
    }

    /// Markdown's `_..._` (rather than `*..*`) so Italic never collides
    /// visually with a Bulleted List's `- ` or a lone `*`.
    pub(super) fn toggle_italic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("_", "_", window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_markers_are_replaced_rather_than_stacked() {
        assert_eq!(strip_leading_marker("# Title"), "Title");
        assert_eq!(strip_leading_marker("### Sub"), "Sub");
        assert_eq!(strip_leading_marker("- [ ] Buy milk"), "Buy milk");
        assert_eq!(strip_leading_marker("- [x] Done"), "Done");
        assert_eq!(strip_leading_marker("- Item"), "Item");
        assert_eq!(strip_leading_marker("12. Item"), "Item");
        assert_eq!(strip_leading_marker("`Mono`"), "Mono");
        assert_eq!(strip_leading_marker("Plain text"), "Plain text");
    }

    #[test]
    fn current_line_range_finds_the_line_around_the_cursor() {
        let value = "first\nsecond\nthird";
        assert_eq!(current_line_range(value, 0), 0..5);
        assert_eq!(current_line_range(value, 3), 0..5);
        assert_eq!(current_line_range(value, 6), 6..12);
        assert_eq!(current_line_range(value, value.len()), 13..18);
    }
}
