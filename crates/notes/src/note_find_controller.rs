//! In-note Find (⌘F), separate from the note list's Note List Search
//! (⌥⌘F, `search_controller.rs`). Case-insensitive by default, matching the
//! Mac's Find (docs/parity.md NOTES-09).

use super::*;

/// Byte offset of every non-overlapping, case-insensitive match of `needle`
/// in `hay`. ASCII-lowercasing keeps every offset valid in the original
/// text: it never changes a UTF-8 string's byte length.
fn note_find_offsets(hay: &str, needle: &str) -> Vec<usize> {
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

impl NotesView {
    pub(super) fn toggle_note_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_find_open {
            self.close_note_find(window, cx);
            return;
        }
        if !self.is_interactive_ready() || self.session.selected_note().is_none() {
            return;
        }
        self.note_find_open = true;
        self.note_find_current = 0;
        self.recompute_note_find_matches(cx);
        self.note_find_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_note_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.note_find_open = false;
        self.note_find_matches.clear();
        self.body.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn recompute_note_find_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.note_find_input.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        self.note_find_matches = note_find_offsets(&body, &query);
        if self.note_find_current >= self.note_find_matches.len() {
            self.note_find_current = 0;
        }
    }

    fn reveal_current_note_match(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&offset) = self.note_find_matches.get(self.note_find_current) else {
            return;
        };
        let needle_len = self.note_find_input.read(cx).value().len();
        self.body.update(cx, |state, cx| {
            state.set_selected_range(offset..offset + needle_len, cx);
            state.focus(window, cx);
        });
    }

    pub(super) fn note_find_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.note_find_open {
            return;
        }
        self.recompute_note_find_matches(cx);
        if self.note_find_matches.is_empty() {
            return;
        }
        self.note_find_current = (self.note_find_current + 1) % self.note_find_matches.len();
        self.reveal_current_note_match(window, cx);
        cx.notify();
    }

    pub(super) fn note_find_previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.note_find_open {
            return;
        }
        self.recompute_note_find_matches(cx);
        if self.note_find_matches.is_empty() {
            return;
        }
        let count = self.note_find_matches.len();
        self.note_find_current = (self.note_find_current + count - 1) % count;
        self.reveal_current_note_match(window, cx);
        cx.notify();
    }

    pub(super) fn render_note_find_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let query_empty = self.note_find_input.read(cx).value().is_empty();
        let status: SharedString = if query_empty {
            "".into()
        } else if self.note_find_matches.is_empty() {
            "Not found".into()
        } else {
            format!(
                "{} of {}",
                self.note_find_current + 1,
                self.note_find_matches.len()
            )
            .into()
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px(px(EDITOR_INSET))
            .py_1()
            .border_b_1()
            .border_color(row_rule())
            .child(
                div()
                    .w(px(220.0))
                    .child(rmac_ui::SearchField::new(&self.note_find_input).appearance(true)),
            )
            .child(
                Button::new("note-find-prev", "")
                    .icon(IconName::ChevronUp)
                    .ghost()
                    .with_size(Size::XSmall)
                    .tooltip("Previous match")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.note_find_previous(window, cx)),
                    ),
            )
            .child(
                Button::new("note-find-next", "")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .with_size(Size::XSmall)
                    .tooltip("Next match")
                    .on_click(cx.listener(|this, _, window, cx| this.note_find_next(window, cx))),
            )
            .child(
                div()
                    .min_w(px(56.0))
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_secondary())
                    .child(status),
            )
            .child(div().flex_1())
            .child(
                Button::new("note-find-close", "")
                    .icon(IconName::Close)
                    .ghost()
                    .with_size(Size::XSmall)
                    .tooltip("Done")
                    .on_click(cx.listener(|this, _, window, cx| this.close_note_find(window, cx))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_find_matches_are_case_insensitive() {
        assert_eq!(
            note_find_offsets("Hello hello HELLO", "hello"),
            vec![0, 6, 12]
        );
        assert!(note_find_offsets("no query", "").is_empty());
        assert!(note_find_offsets("nothing here", "zzz").is_empty());
    }
}
