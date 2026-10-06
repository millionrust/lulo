//! The rich-text format bar (UIA-07): always shown above the ruler while a
//! document is rich text, matching TextEdit's own bar (design-lab/
//! text-editor-format-bar.html, measured against macOS 26.2 TextEdit at
//! 586×488 — docs/parity.md's 2026-10-06 UI audit). Left to right: B/I/U/S,
//! text and highlight colour wells, font family, style and size, the
//! alignment segment, the list pop-up and the line-spacing pop-up.
//!
//! Family, list and spacing stay dynamic or multi-field pickers, so each
//! opens the existing Fonts/List/Spacing sheet (already real, already
//! tested) rather than duplicate that picking logic inline; style, the
//! colour wells and alignment are small enough to act immediately, as the
//! Mac's own bar does.

use super::super::format_text::Highlight;
use super::*;
use rmac_ui::PopUpButton;

/// A toggle segment (B/I/U/S): small, bordered, highlighted while on.
fn style_toggle(
    id: &'static str,
    label: &'static str,
    accessible: &'static str,
    on: bool,
    cx: &mut Context<EditorView>,
    handler: impl Fn(&mut EditorView, &mut Context<EditorView>) + 'static,
) -> impl IntoElement {
    Button::new(id, label)
        .small()
        .tooltip(accessible)
        .selected(on)
        .on_click(cx.listener(move |this, _, _, cx| handler(this, cx)))
}

fn typeface_label(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    }
}

impl EditorView {
    pub(super) fn render_format_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.rich.read(cx).style_at_selection();
        let ruler = self.paragraph_style(cx);
        let editable = self.text_format_editable();
        let family_label = SharedString::from(
            style
                .family
                .as_deref()
                .unwrap_or(rmac_ui::UI_FONT)
                .to_owned(),
        );
        let size_label = if style.size.fract() == 0.0 {
            format!("{:.0}", style.size)
        } else {
            format!("{:.1}", style.size)
        };
        let spacing_label = if ruler.line_spacing.fract() == 0.0 {
            format!("{:.1}×", ruler.line_spacing)
        } else {
            format!("{:.2}×", ruler.line_spacing)
        };
        let list_label = ruler
            .list
            .map(super::super::format_extras::list_marker_name)
            .unwrap_or("List");
        let text_colour = style
            .color
            .map(rich::palette::to_hsla)
            .unwrap_or(mac::text());
        let highlight_swatch = self.highlight_at_selection(cx);
        let highlight_colour = highlight_swatch
            .and_then(|highlight| highlight.color())
            .map(rich::palette::to_hsla);
        let alignment = ruler.alignment;
        let align_button = |id: &'static str,
                            label: &'static str,
                            accessible: &'static str,
                            value: rich::Alignment,
                            cx: &mut Context<Self>| {
            Button::new(id, label)
                .small()
                .tooltip(accessible)
                .selected(alignment == value)
                .on_click(cx.listener(move |this, _, _, cx| this.set_alignment(value, cx)))
        };
        div()
            .id("format-bar")
            .flex_none()
            .h(px(36.0))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            // B I U S
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(style_toggle(
                        "format-bold",
                        "B",
                        "Bold",
                        style.bold,
                        cx,
                        |this, cx| this.toggle_bold(cx),
                    ))
                    .child(style_toggle(
                        "format-italic",
                        "I",
                        "Italic",
                        style.italic,
                        cx,
                        |this, cx| this.toggle_italic(cx),
                    ))
                    .child(style_toggle(
                        "format-underline",
                        "U",
                        "Underline",
                        style.underline,
                        cx,
                        |this, cx| this.toggle_underline(cx),
                    ))
                    .child(style_toggle(
                        "format-strikethrough",
                        "S",
                        "Strikethrough",
                        style.strikethrough,
                        cx,
                        |this, cx| this.toggle_strikethrough(cx),
                    )),
            )
            // Text colour well
            .child(
                div()
                    .id("format-text-colour")
                    .role(Role::Button)
                    .aria_label("Text Colour")
                    .size(px(20.0))
                    .rounded(px(mac::radius_control()))
                    .border_1()
                    .border_color(mac::separator())
                    .bg(text_colour)
                    .cursor_pointer()
                    .when(editable, |well| {
                        well.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.show_colours(cx);
                        }))
                    }),
            )
            // Highlight colour well: opens the same seven highlights the
            // Format ▸ Font ▸ Highlight menu already offers, labelled with
            // the active one (its own swatch tints the trigger).
            .child(
                PopUpButton::new(
                    "format-highlight",
                    highlight_swatch.map_or("Highlight", |h| h.label()),
                )
                .disabled(!editable)
                .selected(highlight_swatch.is_some_and(|h| h != Highlight::None))
                .dropdown_menu(|menu, _, _| {
                    menu.menu("None", Box::new(HighlightNone))
                        .menu("Accent", Box::new(HighlightAccent))
                        .menu("Purple", Box::new(HighlightPurple))
                        .menu("Pink", Box::new(HighlightPink))
                        .menu("Orange", Box::new(HighlightOrange))
                        .menu("Mint", Box::new(HighlightMint))
                        .menu("Blue", Box::new(HighlightBlue))
                })
                .when_some(highlight_colour, |well, colour| well.bg(colour)),
            )
            // Font family: opens the Fonts panel (⌘T), already a live,
            // scanned list — a second inline copy of that list would be a
            // second, divergent implementation of the same picker.
            .child(
                Button::new("format-font-family", family_label)
                    .small()
                    .tooltip("Font")
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, window, cx| this.show_fonts(window, cx))),
            )
            // Style (Regular/Bold/Italic/Bold Italic): a fixed four-item
            // pop-up distinct from the B/I toggles beside it.
            .child(
                PopUpButton::new("format-typeface", typeface_label(style.bold, style.italic))
                    .disabled(!editable)
                    .dropdown_menu(|menu, _, _| {
                        menu.menu("Regular", Box::new(TypefaceRegular))
                            .menu("Bold", Box::new(TypefaceBold))
                            .menu("Italic", Box::new(TypefaceItalic))
                            .menu("Bold Italic", Box::new(TypefaceBoldItalic))
                    }),
            )
            // Size: the selection's point size with a stepper, the same
            // Bigger/Smaller step Format ▸ Font already applies.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("format-size-down", "−")
                            .xsmall()
                            .tooltip("Smaller")
                            .disabled(!editable)
                            .on_click(cx.listener(|this, _, _, cx| this.smaller(cx))),
                    )
                    .child(
                        div()
                            .w(px(22.0))
                            .text_center()
                            .text_size(rmac_ui::text_px(12.0))
                            .child(size_label),
                    )
                    .child(
                        Button::new("format-size-up", "+")
                            .xsmall()
                            .tooltip("Bigger")
                            .disabled(!editable)
                            .on_click(cx.listener(|this, _, _, cx| this.bigger(cx))),
                    ),
            )
            // Alignment
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(align_button(
                        "format-align-left",
                        "⟸",
                        "Align Left",
                        rich::Alignment::Left,
                        cx,
                    ))
                    .child(align_button(
                        "format-align-centre",
                        "≡",
                        "Centre",
                        rich::Alignment::Center,
                        cx,
                    ))
                    .child(align_button(
                        "format-align-right",
                        "⟹",
                        "Align Right",
                        rich::Alignment::Right,
                        cx,
                    ))
                    .child(align_button(
                        "format-align-justify",
                        "☰",
                        "Justify",
                        rich::Alignment::Justified,
                        cx,
                    )),
            )
            // List: opens the List… sheet (Format ▸ List…), which already
            // covers every marker kind.
            .child(
                Button::new("format-list", list_label)
                    .small()
                    .tooltip("List")
                    .selected(ruler.list.is_some())
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, _, cx| this.open_lists(cx))),
            )
            // Line spacing: opens the Spacing… sheet (Format ▸ Text ▸
            // Spacing…), which already covers every multiple.
            .child(
                Button::new("format-spacing", spacing_label)
                    .small()
                    .tooltip("Line Spacing")
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, _, cx| this.open_spacing(cx))),
            )
    }
}
