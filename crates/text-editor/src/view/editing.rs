//! Text Editor Find/Replace, typography, encoding, and line-ending commands.

use super::*;

impl EditorView {
    pub(super) fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && !self.replace_mode {
            self.close_bar(cx);
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
            self.close_bar(cx);
        } else {
            self.find_open = true;
            self.replace_mode = true;
            self.current = 0;
            self.recompute_matches(cx);
            self.find_input
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn close_bar(&mut self, cx: &mut Context<Self>) {
        self.find_open = false;
        self.replace_mode = false;
        cx.notify();
    }

    /// Case-sensitive scan of the buffer for the current query, recording the
    /// byte offset of every match.
    pub(super) fn recompute_matches(&mut self, cx: &Context<Self>) {
        let needle = self.find_input.read(cx).value().to_string();
        let hay = self.input.read(cx).value().to_string();
        let mut matches = Vec::new();
        if !needle.is_empty() {
            let mut start = 0;
            while let Some(position) = hay[start..].find(&needle) {
                let absolute = start + position;
                matches.push(absolute);
                start = absolute + needle.len();
            }
        }
        if self.current >= matches.len() {
            self.current = 0;
        }
        self.matches = matches;
    }

    fn scroll_to_current(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(&offset) = self.matches.get(self.current) {
            let position: Position = self.input.read(cx).text().offset_to_position(offset);
            self.input.update(cx, |state, cx| {
                state.set_cursor_position(position, window, cx)
            });
        }
    }

    pub(super) fn find_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.scroll_to_current(window, cx);
        cx.notify();
    }

    pub(super) fn find_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len();
        self.current = (self.current + count - 1) % count;
        self.scroll_to_current(window, cx);
        cx.notify();
    }

    pub(super) fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            return;
        }
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let offset = self.matches[self.current];
        let needle = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let mut hay = self.input.read(cx).value().to_string();
        if offset + needle.len() <= hay.len()
            && &hay[offset..offset + needle.len()] == needle.as_str()
        {
            hay.replace_range(offset..offset + needle.len(), &replacement);
            self.input
                .update(cx, |state, cx| state.set_value(hay, window, cx));
            self.on_buffer_changed(cx);
            self.scroll_to_current(window, cx);
            cx.notify();
        }
    }

    pub(super) fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            return;
        }
        let needle = self.find_input.read(cx).value().to_string();
        if needle.is_empty() {
            return;
        }
        let replacement = self.replace_input.read(cx).value().to_string();
        let hay = self.input.read(cx).value().to_string();
        if !hay.contains(&needle) {
            return;
        }
        let value = hay.replace(&needle, &replacement);
        self.input
            .update(cx, |state, cx| state.set_value(value, window, cx));
        self.current = 0;
        self.on_buffer_changed(cx);
        cx.notify();
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
}
