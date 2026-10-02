//! Text Editor Find/Replace, typography, encoding, and line-ending commands.

use gpui::EntityInputHandler as _;

use super::*;

/// Largest document whose text is exposed as the accessibility tree's value.
/// The value is cached per text revision (see
/// [`EditorView::accessible_document_value`]), so this bounds one copy per
/// edit, not per frame; a bigger document keeps its role and name but
/// exposes no value.
const MAX_ACCESSIBLE_VALUE_BYTES: usize = 1024 * 1024;

fn accessible_value_fits(len_bytes: usize) -> bool {
    len_bytes <= MAX_ACCESSIBLE_VALUE_BYTES
}

/// A one-based logical line, excluding its line break. The final empty line
/// after a trailing newline is a valid destination.
fn line_number_range(text: &str, number: usize) -> Option<std::ops::Range<usize>> {
    if number == 0 {
        return None;
    }
    let mut start = 0;
    for _ in 1..number {
        start += text[start..].find('\n')? + 1;
    }
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index);
    Some(start..end)
}

/// Byte offset of every non-overlapping match of `needle`, case-insensitive
/// as the Mac's Find is by default. Comparing ASCII-lowercased copies keeps
/// every byte offset valid in the original text: lowercasing never changes a
/// UTF-8 string's byte length.
fn match_offsets(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let hay_lower = hay.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    hay_lower
        .match_indices(&needle_lower)
        .map(|(offset, _)| offset)
        .collect()
}

/// Whether the bytes at `offset..offset+needle.len()` in `hay` are still the
/// same case-insensitive match `match_offsets` found there.
fn matches_needle_at(hay: &str, offset: usize, needle: &str) -> bool {
    offset + needle.len() <= hay.len()
        && hay.is_char_boundary(offset)
        && hay.is_char_boundary(offset + needle.len())
        && hay[offset..offset + needle.len()].eq_ignore_ascii_case(needle)
}

/// `hay` with every `needle_len`-byte span at `offsets` (as `match_offsets`
/// found them) replaced by `replacement`, left to right.
fn replace_at_offsets(
    hay: &str,
    offsets: &[usize],
    needle_len: usize,
    replacement: &str,
) -> String {
    let mut result = String::with_capacity(hay.len());
    let mut cursor = 0;
    for &offset in offsets {
        if offset < cursor {
            continue;
        }
        result.push_str(&hay[cursor..offset]);
        result.push_str(replacement);
        cursor = offset + needle_len;
    }
    result.push_str(&hay[cursor..]);
    result
}

impl EditorView {
    pub(super) fn use_selection_for_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = {
            let state = self.input.read(cx);
            let range = state.selected_range();
            let text = state.text().to_string();
            let Some(selection) = text.get(range).filter(|selection| !selection.is_empty()) else {
                return;
            };
            selection.to_owned()
        };
        self.find_input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.current = 0;
        self.recompute_matches(cx);
        cx.notify();
    }

    pub(super) fn jump_to_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.input.read(cx).selected_range();
        if range.is_empty() {
            return;
        }
        let position = self.input.read(cx).text().offset_to_position(range.start);
        self.input.update(cx, |input, cx| {
            input.set_cursor_position(position, window, cx);
            input.set_selected_range(range, cx);
            input.focus(window, cx);
        });
    }

    pub(super) fn select_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rtf_runs.is_some() || self.long_lines.is_some() {
            return;
        }
        let current_line = {
            let input = self.input.read(cx);
            let text = input.text().to_string();
            text[..input.selected_range().start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1
        };
        self.select_line_input.update(cx, |input, cx| {
            input.set_value(current_line.to_string(), window, cx);
            input.focus(window, cx);
        });
        self.find_open = false;
        self.select_line_open = true;
        cx.notify();
    }

    pub(super) fn select_requested_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.select_line_open {
            return;
        }
        let requested = self.select_line_input.read(cx).value().trim().parse().ok();
        let text = self.input.read(cx).text().to_string();
        let Some(range) = requested.and_then(|number| line_number_range(&text, number)) else {
            return;
        };
        self.select_line_open = false;
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn actual_size(&mut self, cx: &mut Context<Self>) {
        self.font_size = f32::from(crate::settings::current().font_size);
        cx.notify();
    }

    pub(super) fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && !self.replace_mode {
            self.close_bar(window, cx);
        } else {
            self.find_open = true;
            self.replace_mode = false;
            self.current = 0;
            self.recompute_matches(cx);
            self.find_input
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && self.replace_mode {
            self.close_bar(window, cx);
        } else {
            self.find_open = true;
            // The long-line view is read-only: Replace opens plain Find.
            self.replace_mode = self.long_lines.is_none();
            self.current = 0;
            self.recompute_matches(cx);
            self.find_input
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn close_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Escape is bound to this action globally (`Some(CTX)`), so it wins
        // over the Save sheet's own `capture_key_down` — GPUI matches key
        // bindings before raw key-down listeners run. Cancel the sheet here
        // instead of falling through to the find bar (TE-19).
        if matches!(self.alert, Some(ActiveAlert::ConfirmSave(_))) {
            self.alert_cancel(cx);
            return;
        }
        self.find_open = false;
        self.select_line_open = false;
        self.replace_mode = false;
        // Once the field is removed from the tree, its focus handle is no
        // longer under the editor's key context. Return focus to the document
        // so File shortcuts (including Save) continue to dispatch.
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Case-sensitive scan of the buffer for the current query, recording the
    /// byte offset of every match.
    pub(super) fn recompute_matches(&mut self, cx: &Context<Self>) {
        let needle = self.find_input.read(cx).text().to_string();
        let matches = if needle.is_empty() {
            Vec::new()
        } else if let Some(document) = &self.long_lines {
            match_offsets(&document.text, &needle)
        } else {
            match_offsets(&self.input.read(cx).text().to_string(), &needle)
        };
        if self.current >= matches.len() {
            self.current = 0;
        }
        self.matches = matches;
    }

    /// Moves the *document's* own caret/focus to the current match — used
    /// only by Replace (`replace_current`), which is about to edit the
    /// document and should leave the user looking at where. Find itself
    /// (`find_next`/`find_prev`/`submit_find`) keeps focus in the Find
    /// field instead; see [`Self::reveal_current_match`].
    fn scroll_to_current(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(document) = &self.long_lines {
            if let Some(&offset) = self.matches.get(self.current) {
                document.reveal_offset(offset);
            }
            return;
        }
        if let Some(&offset) = self.matches.get(self.current) {
            let position: Position = self.input.read(cx).text().offset_to_position(offset);
            self.input.update(cx, |state, cx| {
                state.set_cursor_position(position, window, cx)
            });
        }
    }

    /// Highlights the current match in the document without moving
    /// keyboard focus there — TextEdit's own Find keeps focus (and the
    /// whole query re-selected, [`Self::reselect_find_query`]) in the Find
    /// field itself across Return/⌘G/⌘⇧G, only ever *showing* the match in
    /// the document.
    fn reveal_current_match(&self, cx: &mut Context<Self>) {
        if let Some(document) = &self.long_lines {
            if let Some(&offset) = self.matches.get(self.current) {
                document.reveal_offset(offset);
            }
            return;
        }
        let Some(&offset) = self.matches.get(self.current) else {
            return;
        };
        let needle_len = self.find_input.read(cx).text().len();
        self.input.update(cx, |state, cx| {
            state.set_selected_range(offset..offset + needle_len, cx)
        });
    }

    /// Re-selects the Find field's whole query, the way TextEdit leaves it
    /// after Return/⌘G/⌘⇧G so retyping immediately replaces it.
    fn reselect_find_query(&self, cx: &mut Context<Self>) {
        let len = self.find_input.read(cx).text().len();
        self.find_input
            .update(cx, |state, cx| state.set_selected_range(0..len, cx));
    }

    /// Return in the Find field: reveals the current match (already the
    /// first one — `recompute_matches` resets `current` to 0 as the query
    /// changes) without moving focus off the field.
    pub(super) fn submit_find(&mut self, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        self.reveal_current_match(cx);
        self.reselect_find_query(cx);
        cx.notify();
    }

    pub(super) fn find_next(&mut self, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.reveal_current_match(cx);
        self.reselect_find_query(cx);
        cx.notify();
    }

    pub(super) fn find_prev(&mut self, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len();
        self.current = (self.current + count - 1) % count;
        self.reveal_current_match(cx);
        self.reselect_find_query(cx);
        cx.notify();
    }

    pub(super) fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy || self.long_lines.is_some() {
            return;
        }
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let offset = self.matches[self.current];
        let needle = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let mut hay = self.input.read(cx).text().to_string();
        if matches_needle_at(&hay, offset, &needle) {
            hay.replace_range(offset..offset + needle.len(), &replacement);
            self.input
                .update(cx, |state, cx| state.set_value(hay, window, cx));
            self.on_buffer_changed(cx);
            self.scroll_to_current(window, cx);
            cx.notify();
        }
    }

    pub(super) fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy || self.long_lines.is_some() {
            return;
        }
        let needle = self.find_input.read(cx).value().to_string();
        if needle.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).value().to_string();
        let hay = self.input.read(cx).text().to_string();
        let offsets = match_offsets(&hay, &needle);
        if offsets.is_empty() {
            return;
        }
        let value = replace_at_offsets(&hay, &offsets, needle.len(), &replacement);
        self.input
            .update(cx, |state, cx| state.set_value(value, window, cx));
        self.current = 0;
        self.on_buffer_changed(cx);
        cx.notify();
    }

    /// The menu bar's live state for this, the key window: Format ▸
    /// Monospaced is ticked while it is on, and Cut, Copy and Delete are
    /// greyed out without a selection in the focused field, as in TextEdit.
    pub(super) fn publish_menu_state(&self, window: &Window, cx: &mut Context<Self>) {
        rmac_ui::set_menu_checked("text_editor::ToggleMono", self.mono, cx);
        rmac_ui::set_menu_label(
            "text_editor::ToggleWrapToPage",
            if self.wrap_to_page {
                "Wrap to Window"
            } else {
                "Wrap to Page"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "text_editor::SaveFile",
            if self.path.is_some() {
                "Save"
            } else {
                "Save…"
            },
            cx,
        );
        let field = [&self.find_input, &self.replace_input]
            .into_iter()
            .find(|field| gpui::Focusable::focus_handle(field.read(cx), cx).is_focused(window))
            .unwrap_or(&self.input);
        let has_selection = !field.read(cx).selected_range().is_empty();
        for action in ["input::Cut", "input::Copy", "input::Delete"] {
            rmac_ui::set_menu_enabled(action, has_selection, cx);
        }
        let has_document_selection = !self.input.read(cx).selected_range().is_empty();
        for action in [
            "text_editor::UseSelectionForFind",
            "text_editor::JumpToSelection",
        ] {
            rmac_ui::set_menu_enabled(action, has_document_selection, cx);
        }
        rmac_ui::set_menu_enabled(
            "text_editor::SelectLine",
            self.rtf_runs.is_none() && self.long_lines.is_none(),
            cx,
        );
    }

    pub(super) fn toggle_mono(&mut self, cx: &mut Context<Self>) {
        self.mono = !self.mono;
        cx.notify();
    }

    pub(super) fn increase_font(&mut self, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + 1.0).min(48.0);
        cx.notify();
    }

    pub(super) fn decrease_font(&mut self, cx: &mut Context<Self>) {
        self.font_size = (self.font_size - 1.0).max(9.0);
        cx.notify();
    }

    pub(super) fn set_encoding(
        &mut self,
        encoding: document::TextEncoding,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        if self.text_format.encoding != encoding {
            self.text_format.encoding = encoding;
            self.refresh_dirty_state(cx);
        }
    }

    pub(super) fn set_line_ending(
        &mut self,
        line_ending: document::LineEnding,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        if !matches!(
            line_ending,
            document::LineEnding::Lf | document::LineEnding::CrLf | document::LineEnding::Cr
        ) {
            return;
        }
        if self.text_format.save_line_ending != line_ending {
            self.text_format.save_line_ending = line_ending;
            self.refresh_dirty_state(cx);
        }
    }

    /// The document text for the accessibility tree, when it is small
    /// enough (see [`MAX_ACCESSIBLE_VALUE_BYTES`]). Built once per text
    /// revision: frames that only move or blink the caret reuse it.
    pub(super) fn accessible_document_value(&mut self, cx: &App) -> Option<SharedString> {
        if let Some((revision, value)) = &self.accessible_value {
            if *revision == self.text_revision {
                return value.clone();
            }
        }
        let value = match &self.long_lines {
            Some(document) => {
                accessible_value_fits(document.text.len()).then(|| document.text.clone())
            }
            None => {
                let text = self.input.read(cx).text();
                accessible_value_fits(text.len()).then(|| SharedString::from(text.to_string()))
            }
        };
        self.accessible_value = Some((self.text_revision, value.clone()));
        value
    }

    /// A listener that applies an assistive technology's text edit to the
    /// buffer through the same undoable path typing uses, so dirty state,
    /// Find matches and autosave follow it.
    pub(super) fn assistive_edit_listener(
        &self,
        edit: AssistiveEdit,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&gpui::accesskit::ActionData>, &mut Window, &mut App) + 'static {
        let view = cx.entity();
        move |data, window, cx| {
            let Some(gpui::accesskit::ActionData::Value(text)) = data else {
                return;
            };
            let text = text.to_string();
            view.update(cx, |this, cx| {
                this.apply_assistive_edit(edit, text, window, cx);
            });
        }
    }

    fn apply_assistive_edit(
        &mut self,
        edit: AssistiveEdit,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recovery_loading
            || self.print_busy
            || self.rtf_runs.is_some()
            || self.long_lines.is_some()
        {
            return;
        }
        self.input.update(cx, |state, cx| match edit {
            AssistiveEdit::SetValue => state.replace_all(text, window, cx),
            AssistiveEdit::ReplaceSelection => state.replace_text_in_range(None, &text, window, cx),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_numbered_line_with_utf8_and_final_blank_line() {
        let text = "one\né💙lan\n";
        assert_eq!(line_number_range(text, 2), Some(4..13));
        assert_eq!(line_number_range(text, 3), Some(14..14));
        assert_eq!(line_number_range(text, 4), None);
        assert_eq!(line_number_range("", 1), Some(0..0));
    }

    #[test]
    fn accessible_value_stops_at_the_copy_limit() {
        assert!(accessible_value_fits(0));
        assert!(accessible_value_fits(MAX_ACCESSIBLE_VALUE_BYTES));
        assert!(!accessible_value_fits(MAX_ACCESSIBLE_VALUE_BYTES + 1));
    }

    #[test]
    fn find_matches_ignore_case_by_default_as_on_the_mac() {
        assert_eq!(match_offsets("Hello HELLO hello", "hello"), vec![0, 6, 12]);
        assert_eq!(match_offsets("Straße", "STRASSE"), Vec::<usize>::new());
        assert!(match_offsets("no query", "").is_empty());
    }

    #[test]
    fn a_stale_match_offset_is_rejected_before_replace() {
        let hay = "Hello world";
        assert!(matches_needle_at(hay, 0, "hello"));
        assert!(matches_needle_at(hay, 6, "WORLD"));
        assert!(!matches_needle_at(hay, 0, "world"));
        assert!(!matches_needle_at(hay, 100, "hello"));
    }

    #[test]
    fn replace_all_is_case_insensitive_and_keeps_the_replacement_case() {
        let hay = "Cat cat CATS";
        let offsets = match_offsets(hay, "cat");
        assert_eq!(
            replace_at_offsets(hay, &offsets, "cat".len(), "dog"),
            "dog dog dogS"
        );
    }
}
