//! Terminal keyboard, clipboard, paste-review, and selection orchestration.

use super::*;

/// Tab and Shift-Tab belong to the shell (completion, reverse completion),
/// as in macOS Terminal. Without these the window root's Tab binding moved
/// keyboard focus to the title-bar buttons, so Tab never reached the shell
/// and focus never came back (found by scripts/a11y/orca_audit.py).
/// `NoAction` in the deeper "Terminal" context disables that binding there,
/// so the key falls through to the grid's own key handler and its encoder.
pub(super) fn shell_owned_key_bindings() -> [KeyBinding; 2] {
    [
        KeyBinding::new("tab", gpui::NoAction, Some("Terminal")),
        KeyBinding::new("shift-tab", gpui::NoAction, Some("Terminal")),
    ]
}

/// Terminal's Copy and Paste keys, kept out of the ordinary cmd-to-ctrl
/// mapping ([`rmac_ui::bind_keys`], used for every other Terminal shortcut
/// below) so a bare Ctrl+<letter> never leaves the shell: Ctrl+C stays the
/// shell's interrupt and PSReadLine keeps its own Ctrl+bindings (ADR 0023).
/// On Windows, Copy and Paste move to Windows Terminal's own Ctrl+Shift+C
/// and Ctrl+Shift+V. Paste Selection — the Mac's ⇧⌘V sibling of Paste —
/// would then collide with Paste's new binding under the ordinary mapping
/// (⇧⌘V already maps to Ctrl+Shift+V), so on Windows only it moves one
/// chord over, to Ctrl+Alt+Shift+V.
pub(super) fn copy_paste_key_bindings() -> [KeyBinding; 3] {
    let context = Some("Terminal");
    if rmac_ui::shortcuts::PRIMARY_IS_CONTROL {
        [
            KeyBinding::new("ctrl-shift-c", Copy, context),
            KeyBinding::new("ctrl-shift-v", Paste, context),
            KeyBinding::new("ctrl-alt-shift-v", PasteSelection, context),
        ]
    } else {
        [
            KeyBinding::new(rmac_ui::shortcuts::COPY.keystroke, Copy, context),
            KeyBinding::new(rmac_ui::shortcuts::PASTE.keystroke, Paste, context),
            KeyBinding::new("shift-cmd-v", PasteSelection, context),
        ]
    }
}

/// A selected manual topic becomes one shell argument, even if it contains
/// spaces or quotes. Control characters are never sent as command input.
pub(super) fn man_command(selection: &str, search_index: bool) -> Option<String> {
    let topic = selection.trim();
    if topic.is_empty() || topic.len() > 256 || topic.chars().any(char::is_control) {
        return None;
    }
    let program = if search_index { "apropos" } else { "man" };
    Some(format!(
        "{program} -- {}\r",
        crate::paste::shell_quote(topic)
    ))
}

impl TerminalView {
    pub(super) fn man_page_for_selection(
        &mut self,
        search_index: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = self
            .selection_text()
            .and_then(|text| man_command(&text, search_index))
        else {
            return;
        };
        if self.modal_open() || self.tabs.len() >= MAX_TABS {
            return;
        }
        let previous_count = self.tabs.len();
        self.new_tab(window, cx);
        if self.tabs.len() == previous_count {
            return;
        }
        if let Err(error) = self.tabs[self.active].write(command.as_bytes()) {
            self.operation_error = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn on_key_down(&mut self, event: &KeyDownEvent) -> Result<bool, SessionWriteError> {
        let mode = {
            let term = self.tabs[self.active]
                .term
                .lock()
                .map_err(|_| SessionWriteError::State)?;
            *term.mode()
        };
        if uses_platform_text_input(&event.keystroke, mode, self.option_as_meta) {
            return Ok(false);
        }
        let kind = if event.is_held {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        let bytes = encode_key_event(&event.keystroke, mode, kind);
        if bytes.is_empty() {
            return Ok(true);
        }
        self.tabs[self.active].write(&bytes)?;
        // Only accepted input jumps to the live prompt and clears the visual
        // selection. A failed writer must not consume local UI state.
        if let Ok(mut term) = self.tabs[self.active].term.lock() {
            term.scroll_display(Scroll::Bottom);
        }
        self.tabs[self.active].ui.selection = None;
        Ok(true)
    }

    pub(super) fn on_key_up(&mut self, event: &KeyUpEvent) -> Result<bool, SessionWriteError> {
        let bytes = {
            let term = self.tabs[self.active]
                .term
                .lock()
                .map_err(|_| SessionWriteError::State)?;
            encode_key_event(&event.keystroke, *term.mode(), KeyEventKind::Release)
        };
        if bytes.is_empty() {
            return Ok(false);
        }
        self.tabs[self.active].write(&bytes)?;
        Ok(true)
    }

    /// Copy the current selection — or, after Edit ▸ Find ▸ Select
    /// All/Select All in Selection, every selected match — to the system
    /// clipboard.
    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.any_selection_text() {
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// Edit ▸ Copy Special ▸ Copy Without Background Colour (⌃⇧⌘C,
    /// TRM-MENU-001): a styled copy of the current drag-selection, kept
    /// distinct from Copy Plain Text above it (both used to call the same
    /// plain `copy`) — RTF and HTML carry the selection's real foreground
    /// ANSI colours (bold/italic/underline too) without each cell's
    /// background fill, so a block of `grep --color` output pastes into
    /// TextEdit, Mail or Notes as coloured text on the page's own
    /// background rather than a block of highlighted colour. Falls back
    /// to a plain copy when the selection is Find's multi-match kind,
    /// which has no per-cell styling to re-render.
    pub(super) fn copy_without_background_colour(&mut self, cx: &mut Context<Self>) {
        if !self.tabs[self.active].ui.selected_matches.is_empty() {
            self.copy(cx);
            return;
        }
        let Some(selection) = self.tabs[self.active].ui.selection else {
            return;
        };
        if selection.is_empty() {
            return;
        }
        let Ok(term) = self.tabs[self.active].term.lock() else {
            return;
        };
        let profile = active();
        let runs =
            crate::copy_special::styled_runs(&selection, &term, self.rows, self.cols, &profile);
        drop(term);
        if runs.is_empty() {
            return;
        }
        let plain: String = runs.iter().map(|run| run.text.as_str()).collect();
        // RTF has no background support here at all (`to_rtf`'s own doc
        // comment), so only HTML needs its background cleared to the
        // page's own colour rather than each cell's.
        let html = crate::copy_special::to_html_without_background(&runs, profile.bg);
        let rtf = crate::copy_special::to_rtf(&runs);
        let metadata = rmac_editor::rich::clipboard::encode_formats(&[
            (rmac_editor::rich::clipboard::RTF_MIME, rtf.as_str()),
            (rmac_editor::rich::clipboard::HTML_MIME, html.as_str()),
        ]);
        cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(plain, metadata));
    }

    /// Paste clipboard text using the active program's exact bracketed-paste
    /// mode. Unprotected multiline content pauses for private-safe review.
    pub(super) fn request_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.request_paste_text(text, window, cx);
    }

    pub(super) fn paste_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            self.request_paste_text(text, window, cx);
        }
    }

    pub(super) fn paste_escaped_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|text| !text.is_empty())
        {
            self.request_paste_text(crate::paste::shell_quote(&text), window, cx);
        }
    }

    pub(super) fn paste_escaped_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text().filter(|text| !text.is_empty()) {
            self.request_paste_text(crate::paste::shell_quote(&text), window, cx);
        }
    }

    fn request_paste_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if text.is_empty() {
            return;
        }
        if text.len() > MAX_PASTE_BYTES {
            self.operation_error =
                Some("Paste exceeds Terminal's 1 MiB input safety limit.".into());
            cx.notify();
            return;
        }
        match self.tabs[self.active].paste(&text, false) {
            Ok(()) => {}
            Err(PasteError::ReviewRequired) => {
                self.pending_paste = Some(PendingPaste::new(self.tabs[self.active].id, text));
                self.capture_active_search_query(cx);
                self.tabs[self.active].ui.search_open = false;
                self.picker_open = false;
                self.menu_at = None;
                window.focus(&self.focus, cx);
            }
            Err(error) => {
                if !matches!(
                    error,
                    PasteError::Session(SessionWriteError::Exited | SessionWriteError::Write)
                ) {
                    self.operation_error = Some(error.to_string().into());
                }
            }
        }
        cx.notify();
    }

    pub(super) fn confirm_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_paste.take() else {
            return;
        };
        let Some(index) = self
            .tabs
            .iter()
            .position(|session| session.id == pending.session_id)
        else {
            self.operation_error = Some("The terminal session changed; nothing was pasted.".into());
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        };
        if index != self.active {
            self.operation_error = Some("The active terminal changed; nothing was pasted.".into());
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if let Err(error) = self.tabs[index].paste(&pending.text, true) {
            if !matches!(
                error,
                PasteError::Session(SessionWriteError::Exited | SessionWriteError::Write)
            ) {
                self.operation_error = Some(error.to_string().into());
            }
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Extract the selected cells as text. Hard line breaks become `\n`, but
    /// soft-wrapped rows (the last cell carries alacritty's `WRAPLINE` flag) are
    /// joined without a newline so a wrapped long line copies as a single line.
    pub(super) fn selection_text(&self) -> Option<String> {
        let selection = self.tabs[self.active].ui.selection?;
        let term = self.tabs[self.active].term.lock().ok()?;
        Some(selection.text(&term, self.rows, self.cols))
    }

    /// Shell ▸ Export Text As…/Print…: the active tab's whole buffer
    /// (scrollback and screen) as plain text.
    pub(super) fn buffer_text(&self) -> Option<String> {
        let term = self.tabs[self.active].term.lock().ok()?;
        Some(crate::ui_state::buffer_text(&term, self.rows, self.cols))
    }

    /// Edit ▸ Find ▸ Select All/Select All in Selection: every selected
    /// match's own text, one per line — what Copy/Export Selected Text As…
    /// use instead of `selection_text` whenever `ui.selected_matches` is
    /// what is actually selected.
    pub(super) fn selected_matches_text(&self) -> Option<String> {
        let matches = &self.tabs[self.active].ui.selected_matches;
        if matches.is_empty() {
            return None;
        }
        let term = self.tabs[self.active].term.lock().ok()?;
        Some(
            matches
                .iter()
                .map(|found| {
                    Selection {
                        anchor: (found.line, found.start),
                        head: (found.line, found.end.saturating_sub(1)),
                    }
                    .text(&term, self.rows, self.cols)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Either kind of "selected text" this tab currently has: Find's
    /// multi-match selection takes precedence when present, otherwise the
    /// plain drag-selection — the two never coexist (`find_select_all`
    /// clears `selection`; starting a drag-selection clears
    /// `selected_matches`).
    pub(super) fn any_selection_text(&self) -> Option<String> {
        self.selected_matches_text()
            .or_else(|| self.selection_text())
    }
}

#[cfg(test)]
mod man_tests {
    use super::man_command;

    #[test]
    fn manual_selection_is_one_quoted_argument() {
        assert_eq!(
            man_command("printf", false).as_deref(),
            Some("man -- printf\r")
        );
        assert_eq!(
            man_command("a'; echo injected", true).as_deref(),
            Some("apropos -- 'a'\\''; echo injected'\r")
        );
        assert!(man_command("bad\ncommand", false).is_none());
    }
}
