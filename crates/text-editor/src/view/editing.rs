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
    /// The document body's selection in UTF-8 bytes (plain field or rich
    /// editor).
    pub(super) fn body_selection(&self, cx: &App) -> std::ops::Range<usize> {
        if self.rich_text {
            self.rich.read(cx).selected_range()
        } else {
            self.input.read(cx).selected_range()
        }
    }

    /// The editable body's text (the read-only long-line view has its own).
    pub(super) fn body_text(&self, cx: &App) -> String {
        if self.rich_text {
            self.rich.read(cx).text()
        } else {
            self.input.read(cx).text().to_string()
        }
    }

    pub(super) fn body_is_empty(&self, cx: &App) -> bool {
        if self.rich_text {
            self.rich.read(cx).document().is_empty()
        } else {
            self.input.read(cx).text().len() == 0 && self.long_lines.is_none()
        }
    }

    /// Select `range` in the body without moving focus.
    fn body_select(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        if self.rich_text {
            self.rich
                .update(cx, |editor, cx| editor.select_range(range, cx));
        } else {
            self.input
                .update(cx, |input, cx| input.set_selected_range(range, cx));
        }
    }

    pub(super) fn use_selection_for_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = {
            let range = self.body_selection(cx);
            let text = self.body_text(cx);
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
        let range = self.body_selection(cx);
        if range.is_empty() {
            return;
        }
        if self.rich_text {
            self.body_select(range, cx);
            self.focus_body(window, cx);
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
        if self.long_lines.is_some() {
            return;
        }
        let current_line = {
            let text = self.body_text(cx);
            let start = self.body_selection(cx).start.min(text.len());
            text.as_bytes()[..start]
                .iter()
                .filter(|byte| **byte == b'\n')
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
        let text = self.body_text(cx);
        let Some(range) = requested.and_then(|number| line_number_range(&text, number)) else {
            return;
        };
        self.select_line_open = false;
        self.body_select(range, cx);
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn transform_selection(
        &mut self,
        transformation: rmac_ui::TextTransformation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused_field = [&self.find_input, &self.replace_input]
            .into_iter()
            .find(|field| gpui::Focusable::focus_handle(field.read(cx), cx).is_focused(window));
        let changed = if let Some(field) = focused_field {
            rmac_ui::transform_selection(field, transformation, window, cx)
        } else if self.editing_blocked() {
            return;
        } else if self.rich_text {
            rmac_ui::transform_selection(&self.rich, transformation, window, cx)
        } else {
            rmac_ui::transform_selection(&self.input, transformation, window, cx)
        };
        if changed {
            cx.notify();
        }
    }

    /// Whether `self.input` currently accepts an edit, for commands that
    /// reach into the document directly (Insert ▸ breaks, Transformations,
    /// Spelling and Grammar, Substitutions).
    pub(super) fn editing_blocked(&self) -> bool {
        self.file_busy
            || self.file_action_blocked()
            || self.prevent_editing
            || self.long_lines.is_some()
    }

    pub(super) fn insert_break(
        &mut self,
        text: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recovery_loading
            || self.file_busy
            || self.file_action_blocked()
            || self.prevent_editing
            || self.long_lines.is_some()
        {
            return;
        }
        if self.rich_text {
            self.rich.update(cx, |editor, cx| {
                let range = editor.selected_range();
                editor.replace_range(range, text, cx);
            });
            self.focus_body(window, cx);
            return;
        }
        self.input.update(cx, |state, cx| {
            // The editor's own Enter action uses the silent replacement path
            // for a newline. Use that same undoable path for Insert ▸ breaks.
            state.replace(text, window, cx);
            state.focus(window, cx);
        });
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
            self.replace_mode = self.long_lines.is_none() && !self.prevent_editing;
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
            self.alert_cancel(window, cx);
            return;
        }
        if self.rename_open {
            self.cancel_rename(window, cx);
            return;
        }
        if self.page_setup_open {
            self.cancel_page_setup(window, cx);
            return;
        }
        if self.spacing_open {
            self.close_spacing(window, cx);
            return;
        }
        self.find_open = false;
        self.select_line_open = false;
        self.replace_mode = false;
        // Once the field is removed from the tree, its focus handle is no
        // longer under the editor's key context. Return focus to the document
        // so File shortcuts (including Save) continue to dispatch.
        self.focus_body(window, cx);
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
            match_offsets(&self.body_text(cx), &needle)
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
        if self.rich_text {
            if let Some(&offset) = self.matches.get(self.current) {
                self.rich
                    .update(cx, |editor, cx| editor.select_range(offset..offset, cx));
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
        if self.rich_text {
            self.rich.update(cx, |editor, cx| {
                editor.select_range(offset..offset + needle_len, cx)
            });
            return;
        }
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
        if self.print_busy || self.prevent_editing || self.long_lines.is_some() {
            return;
        }
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let offset = self.matches[self.current];
        let needle = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        if self.rich_text {
            let hay = self.body_text(cx);
            if matches_needle_at(&hay, offset, &needle) {
                self.rich.update(cx, |editor, cx| {
                    editor.replace_range(offset..offset + needle.len(), &replacement, cx)
                });
                self.scroll_to_current(window, cx);
                cx.notify();
            }
            return;
        }
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
        if self.print_busy || self.prevent_editing || self.long_lines.is_some() {
            return;
        }
        let needle = self.find_input.read(cx).value().to_string();
        if needle.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).value().to_string();
        let hay = self.body_text(cx);
        let offsets = match_offsets(&hay, &needle);
        if offsets.is_empty() {
            return;
        }
        if self.rich_text {
            let ranges: Vec<std::ops::Range<usize>> = offsets
                .iter()
                .map(|offset| *offset..*offset + needle.len())
                .collect();
            self.rich.update(cx, |editor, cx| {
                editor.replace_ranges(&ranges, &replacement, cx)
            });
            self.current = 0;
            cx.notify();
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
        rmac_ui::set_menu_enabled(
            "text_editor::CloseAll",
            startup::document_window_count() > 1,
            cx,
        );
        rmac_ui::set_menu_label(
            "text_editor::EnterFullScreen",
            if window.is_fullscreen() {
                "Exit Full Screen"
            } else {
                "Enter Full Screen"
            },
            cx,
        );
        rmac_ui::set_menu_checked("text_editor::ToggleMono", self.mono, cx);
        rmac_ui::set_menu_checked("text_editor::PreventEditing", self.prevent_editing, cx);
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
        let focused_field = [&self.find_input, &self.replace_input]
            .into_iter()
            .find(|field| gpui::Focusable::focus_handle(field.read(cx), cx).is_focused(window));
        let has_document_selection = !self.body_selection(cx).is_empty();
        let has_selection = match focused_field {
            Some(field) => !field.read(cx).selected_range().is_empty(),
            None => has_document_selection,
        };
        for action in ["input::Cut", "input::Copy", "input::Delete"] {
            rmac_ui::set_menu_enabled(action, has_selection, cx);
        }
        for action in [
            "text_editor::UseSelectionForFind",
            "text_editor::JumpToSelection",
        ] {
            rmac_ui::set_menu_enabled(action, has_document_selection, cx);
        }
        rmac_ui::set_menu_enabled("text_editor::SelectLine", self.long_lines.is_none(), cx);
        let can_transform = has_selection
            && (focused_field.is_some()
                || (!self.file_busy
                    && !self.file_action_blocked()
                    && !self.prevent_editing
                    && self.long_lines.is_none()));
        for action in [
            "text_editor::TransformUppercase",
            "text_editor::TransformLowercase",
            "text_editor::TransformCapitalise",
        ] {
            rmac_ui::set_menu_enabled(action, can_transform, cx);
        }
        let can_insert = !self.recovery_loading
            && !self.file_busy
            && !self.file_action_blocked()
            && !self.prevent_editing
            && self.long_lines.is_none();
        for action in [
            "text_editor::InsertLineBreak",
            "text_editor::InsertParagraphBreak",
            "text_editor::InsertPageBreak",
        ] {
            rmac_ui::set_menu_enabled(action, can_insert, cx);
        }

        // File ▸ Rename…/Move To…/Revert To ▸ Last Saved (TXT-MENU-002/003/004):
        // only a saved, path-backed document has a name, location or saved
        // revision to act on.
        let has_path = self.path.is_some();
        rmac_ui::set_menu_enabled("text_editor::RenameDocument", has_path, cx);
        rmac_ui::set_menu_enabled("text_editor::MoveToFolder", has_path, cx);
        rmac_ui::set_menu_enabled("text_editor::RevertToLastSaved", has_path && self.dirty, cx);

        // Format ▸ Make Rich Text / Make Plain Text (TXT-MENU-075) and the
        // Format ▸ Text submenu it gates (TXT-MENU-060..074).
        rmac_ui::set_menu_label(
            "text_editor::ToggleRichText",
            if self.rich_text {
                "Make Plain Text"
            } else {
                "Make Rich Text"
            },
            cx,
        );
        // Format ▸ Font's styles and Format ▸ Text belong to rich text; the
        // Mac greys them out in a plain document.
        let text_format_enabled = self.rich_text && can_insert;
        for action in [
            "text_editor::ShowFonts",
            "text_editor::ToggleBold",
            "text_editor::ToggleItalic",
            "text_editor::ToggleUnderline",
            "text_editor::ShowColours",
            "text_editor::CopyStyle",
            "text_editor::HighlightNone",
            "text_editor::HighlightAccent",
            "text_editor::HighlightPurple",
            "text_editor::HighlightPink",
            "text_editor::HighlightOrange",
            "text_editor::HighlightMint",
            "text_editor::HighlightBlue",
            "text_editor::AlignLeft",
            "text_editor::AlignCentre",
            "text_editor::AlignJustify",
            "text_editor::AlignRight",
            "text_editor::ShowRuler",
            "text_editor::CopyRuler",
            "text_editor::PasteRuler",
            "text_editor::OpenSpacing",
            "text_editor::ShowLists",
        ] {
            rmac_ui::set_menu_enabled(action, text_format_enabled, cx);
        }
        rmac_ui::set_menu_enabled(
            "text_editor::PasteStyle",
            text_format_enabled && Self::style_copied(),
            cx,
        );
        self.sync_format_extras_menu(text_format_enabled, cx);
        let (style, paragraph, highlight) = if self.rich_text {
            let editor = self.rich.read(cx);
            (
                editor.style_at_selection(),
                editor.paragraph_style_at_selection(),
                self.highlight_at_selection(cx),
            )
        } else {
            (
                rich::CharStyle::default(),
                rich::ParagraphStyle::default(),
                None,
            )
        };
        let rich_text = self.rich_text;
        rmac_ui::set_menu_checked("text_editor::ToggleBold", rich_text && style.bold, cx);
        rmac_ui::set_menu_checked("text_editor::ToggleItalic", rich_text && style.italic, cx);
        rmac_ui::set_menu_checked(
            "text_editor::ToggleUnderline",
            rich_text && style.underline,
            cx,
        );
        for (action, row) in [
            ("text_editor::HighlightNone", format_text::Highlight::None),
            (
                "text_editor::HighlightAccent",
                format_text::Highlight::Accent,
            ),
            (
                "text_editor::HighlightPurple",
                format_text::Highlight::Purple,
            ),
            ("text_editor::HighlightPink", format_text::Highlight::Pink),
            (
                "text_editor::HighlightOrange",
                format_text::Highlight::Orange,
            ),
            ("text_editor::HighlightMint", format_text::Highlight::Mint),
            ("text_editor::HighlightBlue", format_text::Highlight::Blue),
        ] {
            rmac_ui::set_menu_checked(action, rich_text && highlight == Some(row), cx);
        }
        for (action, alignment) in [
            ("text_editor::AlignLeft", rich::Alignment::Left),
            ("text_editor::AlignCentre", rich::Alignment::Center),
            ("text_editor::AlignJustify", rich::Alignment::Justified),
            ("text_editor::AlignRight", rich::Alignment::Right),
        ] {
            rmac_ui::set_menu_checked(action, rich_text && paragraph.alignment == alignment, cx);
        }
        rmac_ui::set_menu_checked("text_editor::ShowRuler", self.show_ruler, cx);
        rmac_ui::set_menu_checked(
            "text_editor::ToggleDarkBackground",
            self.dark_background,
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
        if self.file_busy || self.rich_text || self.file_action_blocked() {
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
        if self.file_busy || self.rich_text || self.file_action_blocked() {
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
            None if self.rich_text => {
                let editor = self.rich.read(cx);
                accessible_value_fits(editor.document().len())
                    .then(|| SharedString::from(editor.text()))
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
        if self.recovery_loading || self.print_busy || self.long_lines.is_some() {
            return;
        }
        if self.rich_text {
            self.rich.update(cx, |editor, cx| {
                let range = match edit {
                    AssistiveEdit::SetValue => 0..editor.document().len(),
                    AssistiveEdit::ReplaceSelection => editor.selected_range(),
                };
                editor.replace_range(range, &text, cx);
            });
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
