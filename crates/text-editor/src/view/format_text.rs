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

/// A size typed in the Fonts panel, in points.
fn parse_font_size(text: &str) -> Option<f32> {
    let size = text
        .trim()
        .trim_end_matches("pt")
        .trim()
        .parse::<f32>()
        .ok()?;
    (size.is_finite() && (4.0..=288.0).contains(&size)).then_some(size)
}

/// Installed font families, from fontconfig (`fc-list`), sorted and
/// deduplicated. Runs off the UI thread; empty when fontconfig's tool is
/// unavailable.
fn installed_font_families() -> Vec<String> {
    let Ok(output) = std::process::Command::new("fc-list")
        .args([":", "family"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_font_families(&String::from_utf8_lossy(&output.stdout))
}

/// `fc-list : family` lines list a family's localized names separated by
/// commas; the first is its own name. Hidden faces start with a dot.
fn parse_font_families(listing: &str) -> Vec<String> {
    let mut families: Vec<String> = listing
        .lines()
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim().replace("\\-", "-"))
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families
}

#[cfg(test)]
mod font_tests {
    use super::*;

    #[test]
    fn fontconfig_family_lines_become_one_sorted_name_each() {
        let listing =
            "Noto Sans,Noto Sans Regular\nDejaVu Serif\n.Hidden\nnoto sans mono\nDejaVu Serif\n";
        assert_eq!(
            parse_font_families(listing),
            ["DejaVu Serif", "Noto Sans", "noto sans mono"]
        );
    }

    #[test]
    fn font_sizes_must_be_sane_points() {
        assert_eq!(parse_font_size(" 14 "), Some(14.0));
        assert_eq!(parse_font_size("10.5pt"), Some(10.5));
        assert_eq!(parse_font_size("0"), None);
        assert_eq!(parse_font_size("big"), None);
    }
}
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

/// Which run attribute the colours panel (`show_colours`) sets — the format
/// bar's two `AXColorWell`s (UIA-07): text colour and highlight/background
/// colour share the same crayon grid, distinguished only by which well
/// opened it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ColourTarget {
    #[default]
    Text,
    Highlight,
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
    pub(super) fn color(self) -> Option<rich::Rgb> {
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

    /// The format bar's highlight well label (UIA-07).
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "Highlight",
            Self::Accent => "Accent",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
            Self::Orange => "Orange",
            Self::Mint => "Mint",
            Self::Blue => "Blue",
        }
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
            // An empty document has no formatting to lose, so it converts
            // at once; otherwise TextEdit asks first.
            if self.rich.read(cx).document().is_empty() {
                self.perform_make_plain_text(window, cx);
            } else {
                self.alert = Some(ActiveAlert::ConfirmPlainTextConversion);
                cx.notify();
            }
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
        self.fonts_open = false;
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

    /// The format bar's "S" button (UIA-07): the same character attribute
    /// Format ▸ Font ▸ Styles… already shows and the RTF writer already
    /// round-trips (`strikethrough`), just never offered a toggle before.
    pub(super) fn toggle_strikethrough(&mut self, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            self.rich
                .update(cx, |editor, cx| editor.toggle_strikethrough(cx));
        }
    }

    /// The format bar's Style pop-up (UIA-07): Regular/Bold/Italic/Bold
    /// Italic, set as a pair so picking one never leaves the other toggle in
    /// a state the label didn't ask for.
    pub(super) fn set_typeface(&mut self, bold: bool, italic: bool, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let current = self.rich.read(cx).style_at_selection();
        if current.bold != bold {
            self.rich.update(cx, |editor, cx| editor.toggle_bold(cx));
        }
        if current.italic != italic {
            self.rich.update(cx, |editor, cx| editor.toggle_italic(cx));
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

    /// Format ▸ Font ▸ Show Fonts (⌘T): the installed families and a size.
    pub(super) fn show_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        self.fonts_open = !self.fonts_open;
        if !self.fonts_open {
            self.focus_body(window, cx);
            cx.notify();
            return;
        }
        let size = self.rich.read(cx).style_at_selection().size;
        let size = if size.fract() == 0.0 {
            format!("{size:.0}")
        } else {
            format!("{size:.1}")
        };
        self.font_size_input
            .update(cx, |input, cx| input.set_value(size, window, cx));
        if self.font_families.is_none() {
            cx.spawn(async move |this, cx| {
                let scanned = cx
                    .background_executor()
                    .spawn(async { installed_font_families() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let families = if scanned.is_empty() {
                        // No fontconfig tool: GPUI's own font list.
                        let mut names = cx.text_system().all_font_names();
                        names.sort_by_key(|name| name.to_lowercase());
                        names.dedup();
                        names
                    } else {
                        scanned
                    };
                    this.font_families =
                        Some(families.into_iter().map(SharedString::from).collect());
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn close_fonts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_font_size(window, cx);
        self.fonts_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn set_font_family(&mut self, family: SharedString, cx: &mut Context<Self>) {
        if self.text_format_editable() {
            let family = std::sync::Arc::<str>::from(family.as_ref());
            self.rich
                .update(cx, |editor, cx| editor.set_family(Some(family), cx));
        }
    }

    /// Apply the Fonts panel's size field, when it holds a valid size that
    /// differs from the selection's.
    pub(super) fn apply_font_size(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let Some(size) = parse_font_size(&self.font_size_input.read(cx).value()) else {
            return;
        };
        if self.rich.read(cx).style_at_selection().size != size {
            self.rich.update(cx, |editor, cx| editor.set_size(size, cx));
        }
    }

    /// Format ▸ Font ▸ Show Colours (⇧⌘C): always the text-colour well.
    pub(super) fn show_colours(&mut self, cx: &mut Context<Self>) {
        self.show_colours_for(ColourTarget::Text, cx);
    }

    /// The format bar's own two colour wells (UIA-07): the same crayon
    /// grid, applied to whichever attribute's well was clicked.
    pub(super) fn show_colours_for(&mut self, target: ColourTarget, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        if self.colours_open && self.colours_target == target {
            self.colours_open = false;
        } else {
            self.colours_target = target;
            self.colours_open = true;
        }
        cx.notify();
    }

    pub(super) fn close_colours(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.colours_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    pub(super) fn apply_colours_panel_choice(&mut self, color: Option<rich::Rgb>, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        match self.colours_target {
            ColourTarget::Text => {
                self.rich
                    .update(cx, |editor, cx| editor.set_text_color(color, cx));
            }
            ColourTarget::Highlight => {
                self.rich
                    .update(cx, |editor, cx| editor.set_highlight(color, cx));
            }
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
