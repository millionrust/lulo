//! Format ▸ paragraph styles, lists, checklists, fonts and text: commands
//! over the note body's rich-text model (`rmac_editor::rich`). The body is
//! edited as styled text, as on the Mac, and stored as Notes Markdown
//! (`rich::markdown`), so a note written by an earlier Notes keeps reading
//! the same.

use rmac_editor::rich::{
    self, Alignment, CharStyle, Document, ListKind, ListMetrics, Paragraph, ParagraphRole,
    RichTextEditor, WritingDirection,
};

use gpui::App;

use super::*;

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

/// Notes' typography: the face, size and weight of each paragraph style.
pub(super) fn notes_typography(role: ParagraphRole) -> CharStyle {
    match role {
        ParagraphRole::Title => CharStyle {
            bold: true,
            ..CharStyle::with_size(TITLE_SIZE)
        },
        ParagraphRole::Heading => CharStyle {
            bold: true,
            ..CharStyle::with_size(HEADING_SIZE)
        },
        ParagraphRole::Subheading => CharStyle {
            bold: true,
            ..CharStyle::with_size(SUBHEADING_SIZE)
        },
        ParagraphRole::Body => CharStyle::with_size(BODY_SIZE),
        ParagraphRole::Monostyled => CharStyle {
            family: Some(Arc::from("Menlo")),
            ..CharStyle::with_size(MONOSTYLED_SIZE)
        },
    }
}

/// Notes' lists: the marker in the margin and the text 22 points in.
pub(super) const NOTES_LIST_METRICS: ListMetrics = ListMetrics {
    list_indent: 22.0,
    marker_x: 6.0,
    quote_indent: 14.0,
};

/// A stored body as the editor's document.
pub(super) fn body_document(markdown: &str) -> Document {
    rich::markdown::parse(markdown, &notes_typography)
}

/// The editor's document as a stored body.
pub(super) fn body_markdown(document: &Document) -> String {
    rich::markdown::write(document, &notes_typography)
}

pub(super) fn new_body_editor(window: &mut Window, cx: &mut App) -> Entity<RichTextEditor> {
    cx.new(|cx| {
        let mut editor = RichTextEditor::new(window, cx);
        editor.set_default_style(notes_typography(ParagraphRole::Body));
        editor.set_role_typography(std::rc::Rc::new(notes_typography));
        editor.set_list_metrics(NOTES_LIST_METRICS, cx);
        editor.set_column(px(EDITOR_INSET), None, cx);
        editor.set_smart_lists(true);
        editor.set_document(body_document(""), cx);
        editor
    })
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

fn is_checkbox(paragraph: &Paragraph) -> bool {
    paragraph.style().list == Some(ListKind::Checkbox)
}

/// Format ▸ More's checklist commands over a whole document: Tick All and
/// Untick All change every checklist item; Move Ticked to Bottom sorts each
/// run of checklist items (ticked last, otherwise in order); Delete Ticked
/// removes ticked items.
pub(super) fn checklist_bulk(document: &Document, action: ChecklistBulkAction) -> Document {
    let mut paragraphs: Vec<Paragraph> = document
        .paragraphs()
        .iter()
        .map(|paragraph| (**paragraph).clone())
        .collect();
    match action {
        ChecklistBulkAction::TickAll | ChecklistBulkAction::UntickAll => {
            let checked = matches!(action, ChecklistBulkAction::TickAll);
            for paragraph in &mut paragraphs {
                if is_checkbox(paragraph) && paragraph.style().checked != checked {
                    let mut style = paragraph.style();
                    style.checked = checked;
                    *paragraph = paragraph.with_style(style);
                }
            }
        }
        ChecklistBulkAction::DeleteTicked => {
            paragraphs.retain(|paragraph| !(is_checkbox(paragraph) && paragraph.style().checked));
        }
        ChecklistBulkAction::MoveTickedToBottom => {
            let mut start = 0;
            while start < paragraphs.len() {
                if !is_checkbox(&paragraphs[start]) {
                    start += 1;
                    continue;
                }
                let mut end = start + 1;
                while end < paragraphs.len() && is_checkbox(&paragraphs[end]) {
                    end += 1;
                }
                paragraphs[start..end].sort_by_key(|paragraph| paragraph.style().checked);
                start = end;
            }
        }
    }
    Document::from_paragraphs(paragraphs)
}

/// The document with paragraph `index` swapped with its neighbour above
/// (`up`) or below, when both are list items; and where `index` went.
fn moved_list_item(document: &Document, index: usize, up: bool) -> Option<(Document, usize)> {
    let other = if up {
        index.checked_sub(1)?
    } else {
        index + 1
    };
    if other >= document.paragraph_count()
        || document.paragraph(index).style().list.is_none()
        || document.paragraph(other).style().list.is_none()
    {
        return None;
    }
    let mut paragraphs: Vec<Paragraph> = document
        .paragraphs()
        .iter()
        .map(|paragraph| (**paragraph).clone())
        .collect();
    paragraphs.swap(index, other);
    Some((Document::from_paragraphs(paragraphs), other))
}

impl NotesView {
    /// Whether the body accepts an edit right now.
    pub(super) fn body_format_editable(&self) -> bool {
        self.is_interactive_ready()
            && !self.markdown_preview_visible
            && !self.selected_note_closed()
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted)
    }

    pub(super) fn body_text(&self, cx: &App) -> String {
        self.body.read(cx).text()
    }

    pub(super) fn body_selection(&self, cx: &App) -> std::ops::Range<usize> {
        self.body.read(cx).selected_range()
    }

    pub(super) fn body_focused(&self, window: &Window, cx: &App) -> bool {
        self.body.read(cx).is_focused(window)
    }

    pub(super) fn focus_body(&self, window: &mut Window, cx: &mut App) {
        self.body.read(cx).focus(window, cx);
    }

    /// Run a formatting change on the body, then keep it focused. The
    /// editor reports the change, which schedules the save.
    fn format_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut RichTextEditor, &mut Context<RichTextEditor>),
    ) {
        if !self.body_format_editable() {
            return;
        }
        self.body.update(cx, |editor, cx| {
            change(editor, cx);
            editor.focus(window, cx);
        });
        cx.notify();
    }

    /// Whether the caret's paragraph has a style Convert to Text removes.
    pub(super) fn current_line_has_structure(&self, cx: &App) -> bool {
        let style = self.body.read(cx).paragraph_style_at_selection();
        style.list.is_some() || style.quote || style.role != ParagraphRole::Body
    }

    /// The caret paragraph's alignment, for the menu's checkmarks.
    pub(super) fn current_alignment(&self, cx: &App) -> Alignment {
        self.body.read(cx).paragraph_style_at_selection().alignment
    }

    pub(super) fn insert_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            let selection = editor.selected_range();
            let text = editor.text();
            let needs_newline = selection.start > 0 && !text[..selection.start].ends_with('\n');
            let template = format!(
                "{}| Column 1 | Column 2 |\n| --- | --- |\n|  |  |\n|  |  |\n",
                if needs_newline { "\n" } else { "" }
            );
            let first = selection.start + usize::from(needs_newline) + 2;
            editor.replace_range(selection, &template, cx);
            editor.select_range(first..first + "Column 1".len(), cx);
        });
    }

    pub(super) fn transform_selection(
        &mut self,
        transform: TextTransform,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            let selection = editor.selected_range();
            let text = editor.document().slice(selection.clone());
            let next = transformed_text(&text, transform);
            if !text.is_empty() && next != text {
                editor.replace_range(selection.clone(), &next, cx);
                editor.select_range(selection.start..selection.start + next.len(), cx);
            }
        });
    }

    /// Format ▸ Move Item ▸ Up / Down.
    pub(super) fn move_current_list_item(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            let document = editor.document().clone();
            let caret = editor.cursor();
            let (index, local) = document.locate(caret);
            if let Some((moved, to)) = moved_list_item(&document, index, up) {
                let at = moved.paragraph_start(to) + local;
                editor.replace_document(moved, at..at, cx);
            }
        });
    }

    pub(super) fn apply_checklist_bulk(
        &mut self,
        action: ChecklistBulkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            let updated = checklist_bulk(editor.document(), action);
            let caret = editor.cursor();
            editor.replace_document(updated, caret..caret, cx);
        });
    }

    /// Format ▸ Title / Heading / Subheading / Body / Monostyled.
    pub(super) fn set_paragraph_role(
        &mut self,
        role: ParagraphRole,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| editor.set_paragraph_role(role, cx));
    }

    /// Format ▸ Convert to Text: the paragraph becomes plain Body text.
    pub(super) fn convert_to_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            editor.set_list_marker(None, cx);
            let style = editor.paragraph_style_at_selection();
            if style.quote {
                editor.toggle_quote(cx);
            }
            editor.set_paragraph_role(ParagraphRole::Body, cx);
        });
    }

    /// Format ▸ Bulleted / Dashed / Numbered List and Checklist: on, or
    /// off again when the paragraph already is that kind of list.
    pub(super) fn toggle_list(
        &mut self,
        kind: ListKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            let current = editor.paragraph_style_at_selection().list;
            editor.set_list_marker((current != Some(kind)).then_some(kind), cx);
        });
    }

    pub(super) fn insert_block_quote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.toggle_quote(cx));
    }

    /// Format ▸ Indentation ▸ Increase / Decrease.
    pub(super) fn change_indent(
        &mut self,
        increase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            editor.change_indent(if increase { 1 } else { -1 }, cx)
        });
    }

    /// Edit ▸ Paste and Match Style: the clipboard's text in the style at
    /// the insertion point.
    pub(super) fn paste_plain_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.format_body(window, cx, |editor, cx| {
            let selection = editor.selected_range();
            editor.replace_range(selection, &text, cx);
        });
    }

    /// Edit ▸ Paste and Retain Style: the clipboard with its own styles.
    pub(super) fn paste_retaining_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.paste_from_clipboard(cx));
    }

    pub(super) fn toggle_bold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.toggle_bold(cx));
    }

    pub(super) fn toggle_italic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.toggle_italic(cx));
    }

    pub(super) fn toggle_strikethrough(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.toggle_strikethrough(cx));
    }

    pub(super) fn toggle_underline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.toggle_underline(cx));
    }

    /// Format ▸ Font ▸ Highlight (⇧⌘E).
    pub(super) fn toggle_highlight(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            let on = editor.style_at_selection().highlight.is_none();
            editor.set_highlight(on.then_some(rich::markdown::NOTES_HIGHLIGHT), cx);
        });
    }

    /// Format ▸ Font ▸ Bigger / Smaller (⌘+ / ⌘−): the selection's own
    /// size, a point at a time.
    pub(super) fn change_font_size(
        &mut self,
        delta: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| editor.change_size(delta, cx));
    }

    pub(super) fn toggle_superscript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            if editor.style_at_selection().superscript > 0 {
                editor.reset_baseline(cx);
            } else {
                editor.reset_baseline(cx);
                editor.adjust_superscript(1, cx);
            }
        });
    }

    pub(super) fn toggle_subscript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            if editor.style_at_selection().superscript < 0 {
                editor.reset_baseline(cx);
            } else {
                editor.reset_baseline(cx);
                editor.adjust_superscript(-1, cx);
            }
        });
    }

    pub(super) fn baseline_use_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| editor.reset_baseline(cx));
    }

    /// Format ▸ Font ▸ Remove Style: the selection back to its paragraph
    /// style's plain face (links stay).
    pub(super) fn remove_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            let base = notes_typography(editor.paragraph_role());
            editor.apply_char_style(base, cx);
        });
    }

    /// Format ▸ Font ▸ Copy Style (⌥⌘C).
    pub(super) fn copy_style(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let mut style = self.body.read(cx).style_at_selection();
        style.link = None;
        self.copied_style = Some(style);
    }

    /// Format ▸ Font ▸ Paste Style (⌥⌘V).
    pub(super) fn paste_style(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(copied) = self.copied_style.clone() else {
            return;
        };
        self.format_body(window, cx, |editor, cx| editor.apply_char_style(copied, cx));
    }

    /// Format ▸ Text ▸ Align Left / Centre / Justify / Align Right.
    pub(super) fn set_text_alignment(
        &mut self,
        alignment: Alignment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| editor.set_alignment(alignment, cx));
    }

    /// Format ▸ Text ▸ Writing Direction ▸ Paragraph.
    pub(super) fn set_paragraph_direction(
        &mut self,
        direction: WritingDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            editor.set_writing_direction(direction, cx)
        });
    }

    /// Format ▸ Text ▸ Writing Direction ▸ Selection.
    pub(super) fn set_selection_direction(
        &mut self,
        direction: WritingDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_body(window, cx, |editor, cx| {
            editor.set_selection_direction(direction, cx)
        });
    }

    /// Format ▸ Checklist (⇧⌘L).
    pub(super) fn insert_checklist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_list(ListKind::Checkbox, window, cx);
    }

    /// Format ▸ Mark as Ticked (⇧⌘U): tick the checklist item, or make the
    /// paragraph an unticked one first.
    pub(super) fn toggle_checklist_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.format_body(window, cx, |editor, cx| {
            if editor.paragraph_style_at_selection().list == Some(ListKind::Checkbox) {
                editor.toggle_checked(cx);
            } else {
                editor.set_list_marker(Some(ListKind::Checkbox), cx);
            }
        });
        self.apply_auto_sort_ticked_items(window, cx);
    }

    /// Notes ▸ Settings… ▸ Automatically sort ticked items: Move Ticked to
    /// Bottom after every tick.
    pub(super) fn apply_auto_sort_ticked_items(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.auto_sort_ticked_items {
            self.apply_checklist_bulk(ChecklistBulkAction::MoveTickedToBottom, window, cx);
        }
    }

    /// Clicking a checkbox in Markdown Preview (NOTES-02): flip the
    /// `- [ ] `/`- [x] ` marker of the item at `range` (bytes of the saved
    /// body) and show the result in the editor, undoably.
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
        let mut markdown = body_markdown(self.body.read(cx).document());
        let Some(item) = markdown.get(range.clone()) else {
            return;
        };
        let Some(offset) = item.get(..item.len().min(16)).and_then(|head| {
            ["[ ]", "[x]", "[X]"]
                .iter()
                .find_map(|marker| head.find(marker))
        }) else {
            return;
        };
        let at = range.start + offset;
        let replacement = if &markdown[at..at + 3] == "[ ]" {
            "[x]"
        } else {
            "[ ]"
        };
        markdown.replace_range(at..at + 3, replacement);
        let document = body_document(&markdown);
        self.body.update(cx, |editor, cx| {
            let caret = editor.cursor();
            editor.replace_document(document, caret..caret, cx);
        });
        cx.notify();
        self.apply_auto_sort_ticked_items(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(markdown: &str) -> Document {
        body_document(markdown)
    }

    #[test]
    fn checklist_bulk_actions_preserve_other_paragraphs_and_list_boundaries() {
        let body = document("First\n- [x] one\n- [ ] two\n- [x] three\n\n- [x] four\n- [ ] five");
        assert_eq!(
            body_markdown(&checklist_bulk(
                &body,
                ChecklistBulkAction::MoveTickedToBottom
            )),
            "First\n- [ ] two\n- [x] one\n- [x] three\n\n- [ ] five\n- [x] four"
        );
        assert_eq!(
            body_markdown(&checklist_bulk(&body, ChecklistBulkAction::DeleteTicked)),
            "First\n- [ ] two\n\n- [ ] five"
        );
        assert!(!body_markdown(&checklist_bulk(&body, ChecklistBulkAction::TickAll))
            .contains("[ ]"));
        assert!(!body_markdown(&checklist_bulk(&body, ChecklistBulkAction::UntickAll))
            .contains("[x]"));
    }

    #[test]
    fn moving_one_list_item_swaps_it_with_its_neighbour_only() {
        let body = document("Intro\n- [ ] one\n- [x] two\nOutro");
        let (moved, to) = moved_list_item(&body, 2, true).unwrap();
        assert_eq!(to, 1);
        assert_eq!(body_markdown(&moved), "Intro\n- [x] two\n- [ ] one\nOutro");
        assert!(moved_list_item(&body, 2, false).is_none());
        assert!(moved_list_item(&body, 0, false).is_none());
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

    #[test]
    fn stored_bodies_from_earlier_notes_open_styled() {
        let body = document("# Shopping\n- [ ] Milk\n**Soon**");
        assert_eq!(body.text(), "Shopping\nMilk\nSoon");
        assert_eq!(body.paragraph(0).style().role, ParagraphRole::Title);
        assert_eq!(body.paragraph(0).style_of_char_at(0).size, TITLE_SIZE);
        assert_eq!(body.paragraph(1).style().list, Some(ListKind::Checkbox));
        assert!(body.paragraph(2).style_of_char_at(0).bold);
        // And they are stored again exactly as they were.
        assert_eq!(body_markdown(&body), "# Shopping\n- [ ] Milk\n**Soon**");
    }
}
