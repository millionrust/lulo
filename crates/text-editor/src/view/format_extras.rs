//! Format ▸ Font ▸ Outline / Kern / Ligatures / Baseline / Character
//! Shape, Format ▸ Allow Hyphenation, File ▸ Show Properties and Edit ▸
//! Link… (TE-14, TXT-MENU-001/004/005..023/030).
//!
//! Each is an attribute of the rich document the RTF writer saves and the
//! reader restores; plain text has none of them, so they are greyed out
//! there as on the Mac.

use super::*;
use rmac_editor::rich::{DocumentProperties, Ligatures, ListKind, BASELINE_STEP, KERN_STEP};

/// File ▸ Show Properties' fields, in the sheet's order.
pub(super) const PROPERTY_FIELDS: [&str; 7] = [
    "Author:",
    "Organisation:",
    "Copyright:",
    "Title:",
    "Subject:",
    "Keywords:",
    "Comment:",
];

fn property_slot(properties: &mut DocumentProperties, index: usize) -> &mut String {
    match index {
        0 => &mut properties.author,
        1 => &mut properties.organisation,
        2 => &mut properties.copyright,
        3 => &mut properties.title,
        4 => &mut properties.subject,
        5 => &mut properties.keywords,
        _ => &mut properties.comment,
    }
}

/// The List sheet's spoken name for a marker.
pub(super) fn list_marker_name(kind: ListKind) -> &'static str {
    match kind {
        ListKind::Bullet => "Bullet",
        ListKind::Circle => "Circle",
        ListKind::Square => "Square",
        ListKind::Diamond => "Diamond",
        ListKind::Hyphen => "Hyphen",
        ListKind::Check => "Check",
        ListKind::Numbered => "Numbers",
        ListKind::UpperRoman => "Upper-case Roman numerals",
        ListKind::LowerRoman => "Lower-case Roman numerals",
        ListKind::UpperAlpha => "Upper-case letters",
        ListKind::LowerAlpha => "Lower-case letters",
    }
}

/// The most document styles Format ▸ Font ▸ Styles… pages through.
pub(super) const MAX_DOCUMENT_STYLES: usize = 64;

/// A document style's name in the Styles sheet: "Helvetica 12 pt, Bold,
/// Centred", as the Mac describes a style.
pub(super) fn style_description(style: &rich::CharStyle, ruler: &rich::ParagraphStyle) -> String {
    let size = if style.size.fract() == 0.0 {
        format!("{:.0}", style.size)
    } else {
        format!("{:.1}", style.size)
    };
    let mut parts = vec![format!(
        "{} {size} pt",
        style.family.as_deref().unwrap_or("Helvetica")
    )];
    for (on, name) in [
        (style.bold, "Bold"),
        (style.italic, "Italic"),
        (style.underline, "Underline"),
        (style.strikethrough, "Strikethrough"),
        (style.outline, "Outline"),
        (style.superscript > 0, "Superscript"),
        (style.superscript < 0, "Subscript"),
        (style.color.is_some(), "Colour"),
        (style.highlight.is_some(), "Highlight"),
    ] {
        if on {
            parts.push(name.to_owned());
        }
    }
    match ruler.alignment {
        rich::Alignment::Left => {}
        rich::Alignment::Center => parts.push("Centred".to_owned()),
        rich::Alignment::Right => parts.push("Right".to_owned()),
        rich::Alignment::Justified => parts.push("Justified".to_owned()),
    }
    if let Some(kind) = ruler.list {
        parts.push(format!("{} list", list_marker_name(kind)));
    }
    if ruler.line_spacing != 1.0 {
        parts.push(format!("{}× spacing", ruler.line_spacing));
    }
    parts.join(", ")
}

/// What Edit ▸ Link… stores for a typed address: a bare domain becomes a
/// web address, anything with a scheme stays as typed.
pub(super) fn normalize_link(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let has_scheme = typed.contains("://") || typed.to_ascii_lowercase().starts_with("mailto:");
    if !has_scheme && typed.contains('@') && !typed.contains(char::is_whitespace) {
        return Some(format!("mailto:{typed}"));
    }
    if !has_scheme && typed.contains('.') && !typed.contains(char::is_whitespace) {
        return Some(format!("https://{typed}"));
    }
    Some(typed.to_owned())
}

/// The character-attribute rows Format ▸ Font gains here, for enabling and
/// checkmarks.
pub(super) const CHARACTER_ACTIONS: [&str; 14] = [
    "text_editor::ToggleOutline",
    "text_editor::KernDefault",
    "text_editor::KernNone",
    "text_editor::KernTighten",
    "text_editor::KernLoosen",
    "text_editor::LigaturesDefault",
    "text_editor::LigaturesNone",
    "text_editor::LigaturesAll",
    "text_editor::BaselineDefault",
    "text_editor::BaselineSuperscript",
    "text_editor::BaselineSubscript",
    "text_editor::BaselineRaise",
    "text_editor::BaselineLower",
    "text_editor::ToggleTraditionalForm",
];

impl EditorView {
    fn update_rich(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut rich::RichTextEditor, &mut Context<rich::RichTextEditor>),
    ) {
        if self.text_format_editable() {
            self.rich.update(cx, change);
        }
    }

    pub(super) fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.toggle_outline(cx));
    }

    pub(super) fn kern_default(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.set_kern(None, cx));
    }

    pub(super) fn kern_none(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.set_kern(Some(0.0), cx));
    }

    pub(super) fn kern_tighten(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.adjust_kern(-KERN_STEP, cx));
    }

    pub(super) fn kern_loosen(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.adjust_kern(KERN_STEP, cx));
    }

    pub(super) fn set_ligatures(&mut self, ligatures: Ligatures, cx: &mut Context<Self>) {
        self.update_rich(cx, move |editor, cx| editor.set_ligatures(ligatures, cx));
    }

    pub(super) fn baseline_default(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.reset_baseline(cx));
    }

    pub(super) fn superscript(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.update_rich(cx, move |editor, cx| editor.adjust_superscript(delta, cx));
    }

    pub(super) fn raise_baseline(&mut self, up: bool, cx: &mut Context<Self>) {
        let delta = if up { BASELINE_STEP } else { -BASELINE_STEP };
        self.update_rich(cx, move |editor, cx| editor.adjust_baseline(delta, cx));
    }

    pub(super) fn toggle_traditional_form(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.toggle_traditional(cx));
    }

    /// Format ▸ Allow Hyphenation / Disallow Hyphenation: the document's
    /// lines may break inside words, with a hyphen.
    pub(super) fn toggle_hyphenation(&mut self, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| {
            let mut attributes = editor.document().attributes().clone();
            attributes.hyphenation = !attributes.hyphenation;
            editor.set_document_attributes(attributes, cx);
        });
    }

    pub(super) fn hyphenation_allowed(&self, cx: &App) -> bool {
        self.rich_text && self.rich.read(cx).document().attributes().hyphenation
    }

    /// File ▸ Show Properties (⌥⌘P): the document's properties, kept only
    /// in rich text.
    pub(super) fn show_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let mut properties = self
            .rich
            .read(cx)
            .document()
            .attributes()
            .properties
            .clone();
        for (index, input) in self.property_inputs.clone().iter().enumerate() {
            let value = std::mem::take(property_slot(&mut properties, index));
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.properties_open = true;
        if let Some(first) = self.property_inputs.first() {
            first.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    pub(super) fn commit_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.properties_open {
            return;
        }
        let mut properties = DocumentProperties::default();
        for (index, input) in self.property_inputs.iter().enumerate() {
            *property_slot(&mut properties, index) = input.read(cx).value().trim().to_owned();
        }
        self.update_rich(cx, move |editor, cx| {
            let mut attributes = editor.document().attributes().clone();
            attributes.properties = properties;
            editor.set_document_attributes(attributes, cx);
        });
        self.close_properties(window, cx);
    }

    pub(super) fn close_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.properties_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Format ▸ Font ▸ Styles…: page through the styles the document uses
    /// and apply one to the selection, starting from the selection's own.
    pub(super) fn show_styles(&mut self, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let editor = self.rich.read(cx);
        let mut current = editor.style_at_selection();
        current.link = None;
        let ruler = editor.paragraph_style_at_selection();
        self.styles_index = editor
            .document()
            .styles_in_use(MAX_DOCUMENT_STYLES)
            .iter()
            .position(|(style, paragraph)| *style == current && *paragraph == ruler)
            .unwrap_or(0);
        self.styles_open = true;
        cx.notify();
    }

    pub(super) fn document_styles(&self, cx: &App) -> Vec<(rich::CharStyle, rich::ParagraphStyle)> {
        self.rich
            .read(cx)
            .document()
            .styles_in_use(MAX_DOCUMENT_STYLES)
    }

    pub(super) fn step_style(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.document_styles(cx).len().max(1);
        self.styles_index = if forward {
            (self.styles_index + 1) % count
        } else {
            (self.styles_index + count - 1) % count
        };
        cx.notify();
    }

    /// Apply: the shown style's character and paragraph attributes go to
    /// the selection (the next typing, at an empty selection), as one
    /// undo step each.
    pub(super) fn apply_document_style(&mut self, cx: &mut Context<Self>) {
        let Some((style, ruler)) = self.document_styles(cx).into_iter().nth(self.styles_index)
        else {
            return;
        };
        self.update_rich(cx, move |editor, cx| {
            editor.apply_char_style(style, cx);
            editor.apply_paragraph_style(ruler, cx);
        });
    }

    pub(super) fn close_styles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.styles_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Edit ▸ Link… (⌘K): the selection's link destination.
    pub(super) fn edit_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.text_format_editable() {
            return;
        }
        let current = self
            .rich
            .read(cx)
            .link_at_selection()
            .map(|link| link.to_string())
            .unwrap_or_default();
        self.link_input
            .update(cx, |input, cx| input.set_value(current, window, cx));
        self.link_open = true;
        self.link_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn commit_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.link_open {
            return;
        }
        let typed = self.link_input.read(cx).value().to_string();
        let target = normalize_link(&typed).map(std::sync::Arc::<str>::from);
        self.update_rich(cx, move |editor, cx| editor.set_link(target, cx));
        self.close_link(window, cx);
    }

    pub(super) fn remove_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_rich(cx, |editor, cx| editor.set_link(None, cx));
        self.close_link(window, cx);
    }

    pub(super) fn close_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_open = false;
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Enable and check Format ▸ Font's attribute rows, Format ▸ Allow
    /// Hyphenation, File ▸ Show Properties and Edit ▸ Link….
    pub(super) fn sync_format_extras_menu(&self, text_format_enabled: bool, cx: &mut App) {
        for action in CHARACTER_ACTIONS {
            rmac_ui::set_menu_enabled(action, text_format_enabled, cx);
        }
        for action in [
            "text_editor::ToggleHyphenation",
            "text_editor::ShowProperties",
            "text_editor::EditLink",
            "text_editor::ShowStyles",
        ] {
            rmac_ui::set_menu_enabled(action, text_format_enabled, cx);
        }
        let style = if self.rich_text {
            self.rich.read(cx).style_at_selection()
        } else {
            rich::CharStyle::default()
        };
        let rich_text = self.rich_text;
        rmac_ui::set_menu_checked("text_editor::ToggleOutline", rich_text && style.outline, cx);
        rmac_ui::set_menu_checked(
            "text_editor::ToggleTraditionalForm",
            rich_text && style.traditional,
            cx,
        );
        rmac_ui::set_menu_label(
            "text_editor::ToggleHyphenation",
            if self.hyphenation_allowed(cx) {
                "Disallow Hyphenation"
            } else {
                "Allow Hyphenation"
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_link_addresses_gain_a_scheme_only_when_they_lack_one() {
        assert_eq!(normalize_link("  "), None);
        assert_eq!(
            normalize_link("example.com/page").as_deref(),
            Some("https://example.com/page")
        );
        assert_eq!(
            normalize_link("me@example.com").as_deref(),
            Some("mailto:me@example.com")
        );
        assert_eq!(
            normalize_link("http://example.com").as_deref(),
            Some("http://example.com")
        );
        assert_eq!(
            normalize_link("mailto:a@b.c").as_deref(),
            Some("mailto:a@b.c")
        );
    }

    #[test]
    fn style_descriptions_name_family_size_and_attributes() {
        let style = rich::CharStyle {
            bold: true,
            superscript: 1,
            ..rich::CharStyle::with_size(14.0)
        };
        let ruler = rich::ParagraphStyle {
            alignment: rich::Alignment::Center,
            list: Some(ListKind::Bullet),
            ..rich::ParagraphStyle::default()
        };
        assert_eq!(
            style_description(&style, &ruler),
            "Helvetica 14 pt, Bold, Superscript, Centred, Bullet list"
        );
        assert_eq!(
            style_description(&rich::CharStyle::with_size(10.5), &Default::default()),
            "Helvetica 10.5 pt"
        );
    }

    #[test]
    fn every_property_field_has_its_own_slot() {
        let mut properties = DocumentProperties::default();
        for index in 0..PROPERTY_FIELDS.len() {
            *property_slot(&mut properties, index) = index.to_string();
        }
        assert_eq!(properties.author, "0");
        assert_eq!(properties.organisation, "1");
        assert_eq!(properties.copyright, "2");
        assert_eq!(properties.title, "3");
        assert_eq!(properties.subject, "4");
        assert_eq!(properties.keywords, "5");
        assert_eq!(properties.comment, "6");
    }

    #[test]
    fn every_list_marker_has_a_spoken_name() {
        let names: std::collections::HashSet<_> =
            ListKind::ALL.into_iter().map(list_marker_name).collect();
        assert_eq!(names.len(), ListKind::ALL.len());
    }
}
