//! Format ▸ Make Rich Text / Make Plain Text, Format ▸ Font's character
//! styles, Format ▸ Text (alignment, the ruler, line spacing), Format ▸
//! List…, and View ▸ Use Dark Background for Windows (TXT-MENU-060..075,
//! TXT-MENU-082, TE-03).
//!
//! A rich document is edited in `rmac_editor::rich::RichTextEditor`, an
//! attributed-text editor: bold, italic, underline, sizes, colours and
//! highlights belong to characters, and alignment, spacing and lists to
//! paragraphs, exactly as in TextEdit. Plain text has none of these, so the
//! Mac greys them out there.

use super::*;
use rmac_editor::rich::{Alignment, CharStyle, ListKind, ParagraphStyle};
use std::sync::{Mutex, OnceLock};

/// Format ▸ Text ▸ Copy Ruler / Paste Ruler: one process-wide clipboard, as
/// the Mac's ruler pasteboard is shared across every TextEdit window.
fn ruler_clipboard() -> &'static Mutex<Option<ParagraphStyle>> {
    static CLIPBOARD: OnceLock<Mutex<Option<ParagraphStyle>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(None))
}

/// Format ▸ Font ▸ Copy Style / Paste Style, shared the same way.
fn style_clipboard() -> &'static Mutex<Option<CharStyle>> {
    static CLIPBOARD: OnceLock<Mutex<Option<CharStyle>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(None))
}

/// Format ▸ Font ▸ Highlight's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Highlight {
    None,
    Accent,
    Purple,
    Pink,
    Orange,
    Mint,
    Blue,
}

impl Highlight {
    fn color(self) -> Option<rich::Rgb> {
        let base = match self {
            Self::None => return None,
            Self::Accent | Self::Blue => rmac_ui::mac::system_blue(),
            Self::Purple => rmac_ui::mac::system_purple(),
            Self::Pink => rmac_ui::mac::system_pink(),
            Self::Orange => rmac_ui::mac::system_orange(),
            Self::Mint => rmac_ui::mac::system_teal(),
        };
        let tint = rich::palette::highlight_tint(base);
        // Accent and Blue are separate rows on the Mac; keep their tints
        // apart so each row's checkmark finds its own colour.
        Some(if self == Self::Accent {
            rich::Rgb::new(tint.r, tint.g.saturating_sub(8), tint.b)
        } else {
            tint
        })
    }
}

impl EditorView {
    /// The style a plain document made rich takes, from Settings ▸ New
    /// Document ▸ Rich text font (TXT-SETTINGS-010/014).
    pub(super) fn rich_default_style() -> CharStyle {
        let settings = crate::settings::current();
        let mut style = CharStyle::with_size(f32::from(settings.rich_text_font_size));
        if settings.rich_text_font == crate::settings::RichTextFont::JetBrainsMono {
            style.family = Some(std::sync::Arc::from("Menlo"));
        }
        style
    }

    /// Format ▸ Make Rich Text / Make Plain Text (⇧⌘T, TXT-MENU-075).
    pub(super) fn toggle_rich_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_action_blocked() || self.file_busy || self.prevent_editing {
            return;
        }
        if self.rich_text {
            // TextEdit always asks before a document loses its formatting.
            self.alert = Some(ActiveAlert::ConfirmPlainTextConversion);
            cx.notify();
            return;
        }
        if self.long_lines.is_some() {
            return;
        }
        let text = self.document_text(cx);
        let style = Self::rich_default_style();
        let document = rich::Document::from_plain_text(&text, &style);
        self.input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.rich
            .update(cx, |editor, _| editor.set_default_style(style.clone()));
        self.install_rich_document(document, cx);
        self.saved_rich = None;
        self.show_ruler = crate::settings::current().show_ruler_default;
        self.on_buffer_changed(cx);
        self.focus_body(window, cx);
        cx.notify();
    }

    /// The confirmed Make Plain Text: the text stays, its formatting goes.
    /// An `.rtf` is never overwritten with plain text, so the document
    /// continues untitled, as Edit as Plain Text did.
    pub(super) fn perform_make_plain_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.rich_text {
            return;
        }
        let text = self.rich.read(cx).text();
        self.rich_text = false;
        self.show_ruler = false;
        self.spacing_open = false;
        self.colours_open = false;
        self.lists_open = false;
        self.rich.update(cx, |editor, cx| {
            let style = editor.default_style().clone();
            editor.set_document(rich::Document::empty(&style), cx);
        });
        self.saved_rich = None;
        let longest_line = long_lines::longest_line_bytes(&text);
        self.install_document_text(text, longest_line, window, cx);
        if self.path.as_deref().is_some_and(is_rich_text_path) {
            self.path = None;
            self.saved_bytes = None;
            self.reset_document_watch();
        }
        self.text_format = document::TextFormat::default();
        self.saved_text = Rope::new();
        self.on_buffer_changed(cx);
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Whether Format ▸ Font's styles and Format ▸ Text apply now.
    pub(super) fn text_format_editable(&self) -> bool {
        self.rich_text && !self.file_action_blocked() && !self.prevent_editing && !self.file_busy
    }

    pub(super) fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich.update(cx, |editor, cx| editor.toggle_bold(cx));
        }
    }

    pub(super) fn toggle_italic(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich.update(cx, |editor, cx| editor.toggle_italic(cx));
        }
    }

    pub(super) fn toggle_underline(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich
                .update(cx, |editor, cx| editor.toggle_underline(cx));
        }
    }

    /// Format ▸ Font ▸ Bigger / Smaller: the selection's own sizes in rich
    /// text; the whole document's font in plain text, as in TextEdit.
    pub(super) fn bigger(&mut self, cx: &mut Context<Self>) {
        if self.rich_text {
            if self.text_format_editable() {
                self.rich
                    .update(cx, |editor, cx| editor.change_size(1.0, cx));
            }
        } else {
            self.increase_font(cx);
        }
    }

    pub(super) fn smaller(&mut self, cx: &mut Context<Self>) {
        if self.rich_text {
            if self.text_format_editable() {
                self.rich
                    .update(cx, |editor, cx| editor.change_size(-1.0, cx));
            }
        } else {
            self.decrease_font(cx);
        }
    }

    /// View ▸ Zoom In / Zoom Out / Actual Size: a rich document is scaled
    /// on screen; a plain one changes its display font size.
    pub(super) fn zoom(&mut self, step: Option<f32>, cx: &mut Context<Self>) {
        if self.rich_text {
            self.rich_zoom = match step {
                Some(step) => (self.rich_zoom + step).clamp(0.5, 4.0),
                None => 1.0,
            };
            cx.notify();
        } else {
            match step {
                Some(step) if step > 0.0 => self.increase_font(cx),
                Some(_) => self.decrease_font(cx),
                None => self.actual_size(cx),
            }
        }
    }

    pub(super) fn set_highlight(&mut self, highlight: Highlight, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            let color = highlight.color();
            self.rich
                .update(cx, |editor, cx| editor.set_highlight(color, cx));
        }
    }

    pub(super) fn highlight_at_selection(&self, cx: &App) -> Option<Highlight> {
        let current = self.rich.read(cx).style_at_selection().highlight;
        [
            Highlight::None,
            Highlight::Accent,
            Highlight::Purple,
            Highlight::Pink,
            Highlight::Orange,
            Highlight::Mint,
            Highlight::Blue,
        ]
        .into_iter()
        .find(|highlight| highlight.color() == current)
    }

    /// Format ▸ Font ▸ Show Colours (⇧⌘C).
    pub(super) fn show_colours(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.colours_open = !self.colours_open;
            cx.notify();
        }
    }

    pub(super) fn close_colours(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.colours_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn set_text_colour(&mut self, color: Option<rich::Rgb>, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich
                .update(cx, |editor, cx| editor.set_text_color(color, cx));
        }
    }

    pub(super) fn copy_style(&mut self, cx: &mut Context<Self>) {
        if !self.rich_text {
            return;
        }
        let style = self.rich.read(cx).style_at_selection();
        if let Ok(mut clipboard) = style_clipboard().lock() {
            *clipboard = Some(style);
        }
        cx.notify();
    }

    pub(super) fn paste_style(&mut self, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let copied = style_clipboard()
            .lock()
            .ok()
            .and_then(|clipboard| clipboard.clone());
        if let Some(style) = copied {
            self.rich
                .update(cx, |editor, cx| editor.apply_char_style(style, cx));
        }
    }

    pub(super) fn style_copied() -> bool {
        style_clipboard()
            .lock()
            .is_ok_and(|clipboard| clipboard.is_some())
    }

    pub(super) fn set_alignment(&mut self, alignment: Alignment, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich
                .update(cx, |editor, cx| editor.set_alignment(alignment, cx));
        }
    }

    pub(super) fn paragraph_style(&self, cx: &App) -> ParagraphStyle {
        self.rich.read(cx).paragraph_style_at_selection()
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
        let ruler = self.paragraph_style(cx);
        if let Ok(mut clipboard) = ruler_clipboard().lock() {
            *clipboard = Some(ruler);
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
            self.rich
                .update(cx, |editor, cx| editor.apply_paragraph_style(ruler, cx));
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
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn set_spacing(&mut self, multiplier: f32, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich
                .update(cx, |editor, cx| editor.set_line_spacing(multiplier, cx));
        }
    }

    /// Format ▸ List….
    pub(super) fn open_lists(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.lists_open = true;
            cx.notify();
        }
    }

    pub(super) fn close_lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lists_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn set_list(&mut self, list: Option<ListKind>, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich.update(cx, |editor, cx| editor.set_list(list, cx));
        }
    }

    /// View ▸ Use Dark Background for Windows (TXT-MENU-082): a per-window
    /// paper-colour override, independent of the system appearance.
    pub(super) fn toggle_dark_background(&mut self, cx: &mut Context<Self>) {
        self.dark_background = !self.dark_background;
        cx.notify();
    }
}
