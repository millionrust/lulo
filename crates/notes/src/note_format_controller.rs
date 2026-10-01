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

#[derive(Clone, Copy)]
pub(super) enum ChecklistBulkAction {
    TickAll,
    UntickAll,
    MoveTickedToBottom,
    DeleteTicked,
}

fn checklist_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- [ ] ") || line.starts_with("- [x] ") || line.starts_with("- [X] ")
}

fn ticked_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- [x] ") || line.starts_with("- [X] ")
}

fn checklist_bulk_text(text: &str, action: ChecklistBulkAction) -> String {
    let trailing_newline = text.ends_with('\n');
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    match action {
        ChecklistBulkAction::TickAll | ChecklistBulkAction::UntickAll => {
            for line in &mut lines {
                if !checklist_line(line) {
                    continue;
                }
                if let Some(range) = find_checkbox_marker(line) {
                    line.replace_range(
                        range,
                        if matches!(action, ChecklistBulkAction::TickAll) {
                            "[x]"
                        } else {
                            "[ ]"
                        },
                    );
                }
            }
        }
        ChecklistBulkAction::DeleteTicked => lines.retain(|line| !ticked_line(line)),
        ChecklistBulkAction::MoveTickedToBottom => {
            let mut start = 0;
            while start < lines.len() {
                if !checklist_line(&lines[start]) {
                    start += 1;
                    continue;
                }
                let mut end = start + 1;
                while end < lines.len() && checklist_line(&lines[end]) {
                    end += 1;
                }
                lines[start..end].sort_by_key(|line| ticked_line(line));
                start = end;
            }
        }
    }
    let mut result = lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result
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

/// The byte range of a `- [ ] `/`- [x] `/`- [X] ` checkbox marker within
/// `text`, searched only in a short prefix so a checklist item's own body
/// text can never be mistaken for a marker further in.
fn find_checkbox_marker(text: &str) -> Option<std::ops::Range<usize>> {
    const MARKERS: [&str; 3] = ["[ ]", "[x]", "[X]"];
    let window_end = (0..=text.len().min(16))
        .rev()
        .find(|&index| text.is_char_boundary(index))?;
    let prefix = &text[..window_end];
    MARKERS
        .into_iter()
        .find_map(|marker| prefix.find(marker).map(|index| index..index + marker.len()))
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
    pub(super) fn apply_checklist_bulk(
        &mut self,
        action: ChecklistBulkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.body_format_editable() {
            return;
        }
        let current = self.body.read(cx).value().to_string();
        let updated = checklist_bulk_text(&current, action);
        if updated == current {
            return;
        }
        self.body.update(cx, |state, cx| {
            state.set_selected_range(0..current.len(), cx);
            state.replace(updated, window, cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
    }
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

    pub(super) fn insert_block_quote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_to_current_line(window, cx, |line| {
            if let Some(unquoted) = line.strip_prefix("> ") {
                unquoted.to_string()
            } else {
                format!("> {line}")
            }
        });
    }

    pub(super) fn change_indent(
        &mut self,
        increase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_current_line(window, cx, |line| {
            if increase {
                format!("  {line}")
            } else if let Some(rest) = line.strip_prefix("  ") {
                rest.to_string()
            } else if let Some(rest) = line.strip_prefix(' ') {
                rest.to_string()
            } else {
                line.to_string()
            }
        });
    }

    pub(super) fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let selection = state.selected_range();
            let text = state.value().get(selection).unwrap_or_default().to_string();
            let label = if text.is_empty() { "Link" } else { &text };
            let markdown = format!("[{label}](https://)");
            state.replace(markdown, window, cx);
            let end = state.cursor();
            state.set_selected_range(end - "https://".len() - 1..end - 1, cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
    }

    pub(super) fn paste_plain_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.body.update(cx, |state, cx| {
            state.replace(text, window, cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
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

    /// ⇧⌘U: mark the checklist item on the current line done/not done, like
    /// the Mac. A line that is not yet a checklist item becomes one, not
    /// done, matching Format ▸ Checklist's own insertion text.
    pub(super) fn toggle_checklist_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_to_current_line(window, cx, |line| match find_checkbox_marker(line) {
            Some(marker_range) => {
                let replacement = if &line[marker_range.clone()] == "[ ]" {
                    "[x]"
                } else {
                    "[ ]"
                };
                let mut next = line.to_string();
                next.replace_range(marker_range, replacement);
                next
            }
            None => format!("- [ ] {}", strip_leading_marker(line)),
        });
    }

    /// Clicking a checkbox in Markdown Preview (NOTES-02): flip the
    /// `- [ ] `/`- [x] ` marker at `range` (the whole list item's byte range
    /// in the saved body, from `MarkdownPreviewBlock::source_range`) without
    /// disturbing any other byte offset — `[ ]`/`[x]`/`[X]` are always 3
    /// bytes, so no other range in the document moves. Goes through
    /// `InputState::replace`, so it is undoable like any other edit.
    pub(super) fn toggle_checklist_range(
        &mut self,
        range: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editable = self.is_interactive_ready()
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted);
        if !editable {
            return;
        }
        let mut toggled = false;
        self.body.update(cx, |state, cx| {
            let value = state.value().to_string();
            let Some(item_text) = value.get(range.clone()) else {
                return;
            };
            let Some(marker_range) = find_checkbox_marker(item_text) else {
                return;
            };
            let absolute = range.start + marker_range.start..range.start + marker_range.end;
            let replacement = match value.get(absolute.clone()) {
                Some("[ ]") => "[x]",
                Some("[x]") | Some("[X]") => "[ ]",
                _ => return,
            };
            state.set_selected_range(absolute, cx);
            state.replace(replacement, window, cx);
            toggled = true;
        });
        if toggled {
            self.schedule_current_edit(cx);
            cx.notify();
        }
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

    #[test]
    fn checkbox_marker_is_found_only_near_the_start_of_the_item() {
        assert_eq!(find_checkbox_marker("- [ ] Buy milk"), Some(2..5));
        assert_eq!(find_checkbox_marker("- [x] Done"), Some(2..5));
        assert_eq!(find_checkbox_marker("- [X] Done"), Some(2..5));
        assert_eq!(find_checkbox_marker("- Not a checklist"), None);
        // A literal "[ ]" deep in an item's own text is never mistaken for
        // the marker: the search window is bounded.
        assert_eq!(
            find_checkbox_marker("- A very long line of text before any [ ] appears"),
            None
        );
    }

    #[test]
    fn checklist_bulk_actions_preserve_other_lines_and_list_boundaries() {
        let body = "First\n- [x] one\n- [ ] two\n- [x] three\n\n- [x] four\n- [ ] five\n";
        assert_eq!(
            checklist_bulk_text(body, ChecklistBulkAction::MoveTickedToBottom),
            "First\n- [ ] two\n- [x] one\n- [x] three\n\n- [ ] five\n- [x] four\n"
        );
        assert_eq!(
            checklist_bulk_text(body, ChecklistBulkAction::DeleteTicked),
            "First\n- [ ] two\n\n- [ ] five\n"
        );
        assert!(checklist_bulk_text(body, ChecklistBulkAction::TickAll)
            .lines()
            .filter(|line| checklist_line(line))
            .all(ticked_line));
        assert!(checklist_bulk_text(body, ChecklistBulkAction::UntickAll)
            .lines()
            .filter(|line| checklist_line(line))
            .all(|line| !ticked_line(line)));
    }
}
