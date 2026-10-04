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
    Dashed,
    Numbered,
}

#[derive(Clone, Copy)]
pub(super) enum ChecklistBulkAction {
    TickAll,
    UntickAll,
    MoveTickedToBottom,
    DeleteTicked,
}

#[derive(Clone, Copy)]
pub(super) enum TextTransform {
    Uppercase,
    Lowercase,
    Capitalise,
}

/// Format ▸ Font ▸ Copy Style/Paste Style: which of Notes' five inline
/// Markdown wrap-pairs the copied selection was wrapped in. Notes has no
/// rich-text run model (NOTES-01/03), so "style" here means exactly the
/// marker pairs `note_format_controller.rs` itself can apply — there is no
/// font, size, or colour to copy.
#[derive(Clone, Copy, Default)]
pub(super) struct CopiedStyle {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    underline: bool,
    highlight: bool,
}

/// The five inline wrap-pairs, outermost first, in the fixed nesting order
/// [`CopiedStyle::rebuild`] always produces (bold > italic > strikethrough >
/// underline > highlight > text) so copying and pasting style round-trips
/// exactly for any selection Notes' own commands produced. A selection a
/// person hand-typed in some other nesting order copies correctly (each
/// pair is still detected independently) but may not fully re-wrap on
/// paste, the same bounded, best-effort trade-off `strip_leading_marker`
/// above already makes for paragraph markers.
const STYLE_WRAP_PAIRS: [(&str, &str); 5] = [
    ("**", "**"),
    ("_", "_"),
    ("~~", "~~"),
    ("++", "++"),
    ("==", "=="),
];

fn wrapped_by(text: &str, prefix: &str, suffix: &str) -> bool {
    text.len() >= prefix.len() + suffix.len() && text.starts_with(prefix) && text.ends_with(suffix)
}

fn transformed_text(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalise => {
            let mut word_start = true;
            let mut result = String::new();
            for character in text.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        result.extend(character.to_uppercase());
                    } else {
                        result.extend(character.to_lowercase());
                    }
                    word_start = false;
                } else {
                    result.push(character);
                    word_start = true;
                }
            }
            result
        }
    }
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

fn plain_text_line(line: &str) -> &str {
    strip_leading_marker(line.trim_start_matches("> "))
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

fn is_list_item(line: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with("- ") || line.starts_with("* ") {
        return true;
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

/// Swap one Markdown list line with the adjacent item in its contiguous
/// list. Return the source and new caret, keeping the caret within the item
/// that moved rather than leaving it on its old row.
fn moved_list_item(value: &str, cursor: usize, up: bool) -> Option<(String, usize)> {
    let current = current_line_range(value, cursor);
    let line = value.get(current.clone())?;
    if !is_list_item(line) {
        return None;
    }
    let offset = cursor.saturating_sub(current.start).min(line.len());
    let adjacent = if up {
        let previous_end = current.start.checked_sub(1)?;
        current_line_range(value, previous_end)
    } else {
        let next_start = current.end.checked_add(1)?;
        if next_start > value.len() {
            return None;
        }
        current_line_range(value, next_start)
    };
    let other = value.get(adjacent.clone())?;
    if !is_list_item(other) {
        return None;
    }
    let (start, end, replacement, caret) = if up {
        (
            adjacent.start,
            current.end,
            format!("{line}\n{other}"),
            adjacent.start + offset,
        )
    } else {
        (
            current.start,
            adjacent.end,
            format!("{other}\n{line}"),
            current.start + other.len() + 1 + offset,
        )
    };
    let mut updated = value.to_string();
    updated.replace_range(start..end, &replacement);
    Some((updated, caret))
}

impl NotesView {
    pub(super) fn current_line_has_structure(&self, cx: &Context<Self>) -> bool {
        let body = self.body.read(cx);
        let value = body.value().to_string();
        let Some(line) = value.get(current_line_range(&value, body.cursor())) else {
            return false;
        };
        plain_text_line(line) != line
    }

    /// The current line's Format ▸ Text ▸ Align Left/Centre/Align Right
    /// state, for the menu's live checkmarks.
    pub(super) fn current_line_alignment(
        &self,
        cx: &Context<Self>,
    ) -> rmac_notes_storage::TextAlign {
        let body = self.body.read(cx);
        let value = body.value().to_string();
        let Some(line) = value.get(current_line_range(&value, body.cursor())) else {
            return rmac_notes_storage::TextAlign::Left;
        };
        if line.ends_with(" :center:") {
            rmac_notes_storage::TextAlign::Center
        } else if line.ends_with(" :right:") {
            rmac_notes_storage::TextAlign::Right
        } else {
            rmac_notes_storage::TextAlign::Left
        }
    }

    pub(super) fn insert_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let cursor = state.selected_range().start;
            let value = state.value().to_string();
            let needs_newline = cursor > 0 && !value[..cursor].ends_with('\n');
            let template = format!(
                "{}| Column 1 | Column 2 |\n| --- | --- |\n|  |  |\n|  |  |\n",
                if needs_newline { "\n" } else { "" }
            );
            let first_heading = cursor + usize::from(needs_newline) + 2;
            state.replace(template, window, cx);
            state.set_selected_range(first_heading..first_heading + "Column 1".len(), cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
    }

    pub(super) fn transform_selection(
        &mut self,
        transform: TextTransform,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.body_format_editable() {
            return;
        }
        let mut changed = false;
        self.body.update(cx, |state, cx| {
            let selected = state.selected_range();
            let value = state.value();
            let Some(text) = value.get(selected) else {
                return;
            };
            if text.is_empty() {
                return;
            }
            let next = transformed_text(text, transform);
            if next == text {
                return;
            }
            state.replace(next, window, cx);
            state.focus(window, cx);
            changed = true;
        });
        if changed {
            self.schedule_current_edit(cx);
        }
    }

    pub(super) fn move_current_list_item(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.body_format_editable() {
            return;
        }
        let value = self.body.read(cx).value().to_string();
        let cursor = self.body.read(cx).cursor();
        let Some((updated, caret)) = moved_list_item(&value, cursor, up) else {
            return;
        };
        self.body.update(cx, |state, cx| {
            state.set_selected_range(0..value.len(), cx);
            state.replace(updated, window, cx);
            state.set_selected_range(caret..caret, cx);
            state.focus(window, cx);
        });
        self.schedule_current_edit(cx);
    }

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
    pub(super) fn body_format_editable(&self) -> bool {
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

    /// Turn the current structured Markdown line into an ordinary paragraph.
    pub(super) fn convert_to_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_to_current_line(window, cx, |line| plain_text_line(line).to_string());
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
                ListMarker::Bulleted => format!("* {body}"),
                ListMarker::Dashed => format!("- {body}"),
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

    pub(super) fn toggle_strikethrough(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("~~", "~~", window, cx);
    }

    /// Format ▸ Font ▸ Underline: a `++marker++` span
    /// (`markdown_preview::scan_custom_marker_spans`), the same
    /// non-CommonMark convention Highlight and Baseline use.
    pub(super) fn toggle_underline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("++", "++", window, cx);
    }

    /// Format ▸ Font ▸ Highlight: a `==marker==` span.
    pub(super) fn toggle_highlight(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("==", "==", window, cx);
    }

    /// Format ▸ Font ▸ Baseline ▸ Superscript: a `^marker^` span.
    pub(super) fn toggle_superscript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("^", "^", window, cx);
    }

    /// Format ▸ Font ▸ Baseline ▸ Subscript: a `::marker::` span. Not a
    /// lone `~marker~`: this crate's GFM strikethrough accepts a single
    /// `~` the same as a doubled `~~`, so that span is already claimed.
    pub(super) fn toggle_subscript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_inline_markdown("::", "::", window, cx);
    }

    /// Format ▸ Font ▸ Baseline ▸ Use Default: remove a Superscript or
    /// Subscript wrap from the selection, if either is present.
    pub(super) fn baseline_use_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let range = state.selected_range();
            let value = state.value().to_string();
            let Some(selected) = value.get(range) else {
                return;
            };
            for (prefix, suffix) in [("^", "^"), ("::", "::")] {
                if wrapped_by(selected, prefix, suffix) {
                    let inner = selected[prefix.len()..selected.len() - suffix.len()].to_string();
                    state.replace(inner, window, cx);
                    state.focus(window, cx);
                    return;
                }
            }
        });
        self.schedule_current_edit(cx);
    }

    /// Format ▸ Font ▸ Remove Style: strip every inline Markdown marker
    /// Notes' own Format commands can produce from the selection, leaving
    /// its plain text. Unlike the single-pair helpers above this removes
    /// markers anywhere in the selection, not only at its edges, since a
    /// selection can span several separately styled runs.
    pub(super) fn remove_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |state, cx| {
            let range = state.selected_range();
            let value = state.value().to_string();
            let Some(selected) = value.get(range) else {
                return;
            };
            if selected.is_empty() {
                return;
            }
            let stripped = selected
                .replace("**", "")
                .replace("~~", "")
                .replace("++", "")
                .replace("==", "")
                .replace("::", "")
                .replace('^', "");
            if stripped != selected {
                state.replace(stripped, window, cx);
                state.focus(window, cx);
            }
        });
        self.schedule_current_edit(cx);
    }

    /// Format ▸ Font ▸ Copy Style: record which of the five wrap-pairs the
    /// current selection is wrapped in, for a later Paste Style.
    pub(super) fn copy_style(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let body = self.body.read(cx);
        let value = body.value().to_string();
        let Some(selected) = value.get(body.selected_range()) else {
            return;
        };
        self.copied_style = Some(CopiedStyle {
            bold: wrapped_by(selected, "**", "**"),
            italic: wrapped_by(selected, "_", "_"),
            strikethrough: wrapped_by(selected, "~~", "~~"),
            underline: wrapped_by(selected, "++", "++"),
            highlight: wrapped_by(selected, "==", "=="),
        });
    }

    /// Format ▸ Font ▸ Paste Style: wrap/unwrap the current selection so it
    /// carries exactly the Copy Style selection's set of markers.
    pub(super) fn paste_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let Some(copied) = self.copied_style else {
            return;
        };
        self.body.update(cx, |state, cx| {
            let range = state.selected_range();
            let value = state.value().to_string();
            let Some(selected) = value.get(range) else {
                return;
            };
            if selected.is_empty() {
                return;
            }
            let mut inner = selected.to_string();
            for (prefix, suffix) in STYLE_WRAP_PAIRS {
                if wrapped_by(&inner, prefix, suffix) {
                    inner = inner[prefix.len()..inner.len() - suffix.len()].to_string();
                }
            }
            let mut rebuilt = inner;
            if copied.highlight {
                rebuilt = format!("=={rebuilt}==");
            }
            if copied.underline {
                rebuilt = format!("++{rebuilt}++");
            }
            if copied.strikethrough {
                rebuilt = format!("~~{rebuilt}~~");
            }
            if copied.italic {
                rebuilt = format!("_{rebuilt}_");
            }
            if copied.bold {
                rebuilt = format!("**{rebuilt}**");
            }
            if rebuilt != selected {
                state.replace(rebuilt, window, cx);
                state.focus(window, cx);
            }
        });
        self.schedule_current_edit(cx);
    }

    /// Format ▸ Text ▸ Align Left/Centre/Align Right: a trailing
    /// ` :center:`/` :right:` marker on the current line
    /// (`markdown_preview::strip_trailing_alignment_marker`). There is no
    /// Justify: GPUI's text layout has no justified line-breaking API, so
    /// it is left out of the menu rather than faked (docs/parity.md).
    pub(super) fn set_text_alignment(
        &mut self,
        align: rmac_notes_storage::TextAlign,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_current_line(window, cx, |line| {
            let mut base = line;
            for marker in [" :center:", " :right:"] {
                if let Some(stripped) = base.strip_suffix(marker) {
                    base = stripped;
                    break;
                }
            }
            match align {
                rmac_notes_storage::TextAlign::Left => base.to_string(),
                rmac_notes_storage::TextAlign::Center => format!("{base} :center:"),
                rmac_notes_storage::TextAlign::Right => format!("{base} :right:"),
            }
        });
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
        self.apply_auto_sort_ticked_items(window, cx);
    }

    /// Notes ▸ Settings… ▸ Automatically sort ticked items: reruns Format ▸
    /// More ▸ Move Ticked to Bottom's own transform after any checklist
    /// toggle. Simplification: the caret can land away from the line just
    /// toggled when that line itself was the one moved, same as a Mac
    /// animation moving an item out from under the caret.
    fn apply_auto_sort_ticked_items(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.auto_sort_ticked_items {
            self.apply_checklist_bulk(ChecklistBulkAction::MoveTickedToBottom, window, cx);
        }
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
            self.apply_auto_sort_ticked_items(window, cx);
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
    fn convert_to_text_removes_structural_markers() {
        assert_eq!(plain_text_line("- [x] Buy milk"), "Buy milk");
        assert_eq!(plain_text_line("> ## Heading"), "Heading");
        assert_eq!(plain_text_line("ordinary text"), "ordinary text");
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

    #[test]
    fn moving_one_list_item_preserves_its_caret_and_other_paragraphs() {
        let body = "Intro\n- [ ] one\n- [x] two\nOutro";
        let cursor = body.find("two").unwrap() + 1;
        let (moved, caret) = moved_list_item(body, cursor, true).unwrap();
        assert_eq!(moved, "Intro\n- [x] two\n- [ ] one\nOutro");
        assert_eq!(&moved[caret - 1..caret + 2], "two");
        assert!(moved_list_item(body, cursor, false).is_none());
        assert!(moved_list_item(body, 1, true).is_none());
    }

    #[test]
    fn text_transformations_handle_unicode_and_word_boundaries() {
        assert_eq!(
            transformed_text("élan and café", TextTransform::Uppercase),
            "ÉLAN AND CAFÉ"
        );
        assert_eq!(
            transformed_text("ÉLAN AND CAFÉ", TextTransform::Lowercase),
            "élan and café"
        );
        assert_eq!(
            transformed_text("hELLO-world again", TextTransform::Capitalise),
            "Hello-World Again"
        );
    }
}
