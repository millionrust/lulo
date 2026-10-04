//! Format ▸ Make Rich Text / Make Plain Text, Format ▸ Text (alignment, the
//! ruler, line spacing), and View ▸ Use Dark Background for Windows
//! (TXT-MENU-060..074, TXT-MENU-075, TXT-MENU-082).
//!
//! Lulo's document model has no per-paragraph attributes (TE-03): a rich
//! document's "ruler" — alignment and line spacing — applies to the whole
//! buffer at once, the same simplification `font_size`/`mono` already use
//! for Bigger/Smaller and Monospaced. Character-level formatting (bold,
//! italic, underline, fonts, colours) stays out of this menu; it would need
//! a new attributed-text editor widget (see docs/parity.md TE-03/TE-14).

use super::*;
use std::sync::{Mutex, OnceLock};

/// Format ▸ Text ▸ Copy Ruler / Paste Ruler: one process-wide clipboard, as
/// the Mac's ruler pasteboard is shared across every TextEdit window.
fn ruler_clipboard() -> &'static Mutex<Option<Ruler>> {
    static CLIPBOARD: OnceLock<Mutex<Option<Ruler>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(None))
}

impl EditorView {
    /// Format ▸ Make Rich Text / Make Plain Text (⇧⌘T, TXT-MENU-075).
    pub(super) fn toggle_rich_text(&mut self, cx: &mut Context<Self>) {
        if self.file_action_blocked() || self.rtf_runs.is_some() {
            return;
        }
        self.rich_text = !self.rich_text;
        if !self.rich_text {
            self.show_ruler = false;
            self.spacing_open = false;
        }
        cx.notify();
    }

    fn text_format_editable(&self) -> bool {
        self.rich_text && !self.file_action_blocked() && self.rtf_runs.is_none()
    }

    pub(super) fn set_alignment(&mut self, alignment: gpui::TextAlign, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        self.ruler.alignment = alignment;
        cx.notify();
    }

    pub(super) fn toggle_show_ruler(&mut self, cx: &mut Context<Self>) {
        if !self.rich_text || self.file_action_blocked() {
            return;
        }
        self.show_ruler = !self.show_ruler;
        cx.notify();
    }

    pub(super) fn copy_ruler(&mut self, cx: &mut Context<Self>) {
        if !self.rich_text {
            return;
        }
        if let Ok(mut clipboard) = ruler_clipboard().lock() {
            *clipboard = Some(self.ruler);
        }
        cx.notify();
    }

    pub(super) fn paste_ruler(&mut self, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let copied = ruler_clipboard()
            .lock()
            .ok()
            .and_then(|clipboard| *clipboard);
        if let Some(ruler) = copied {
            self.ruler = ruler;
            cx.notify();
        }
    }

    pub(super) fn open_spacing(&mut self, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        self.spacing_open = true;
        cx.notify();
    }

    pub(super) fn close_spacing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.spacing_open = false;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn set_spacing(&mut self, multiplier: f32, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        self.ruler.line_spacing = multiplier;
        cx.notify();
    }

    /// View ▸ Use Dark Background for Windows (TXT-MENU-082): a per-window
    /// paper-colour override, independent of the system appearance.
    pub(super) fn toggle_dark_background(&mut self, cx: &mut Context<Self>) {
        self.dark_background = !self.dark_background;
        cx.notify();
    }
}
