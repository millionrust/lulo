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
//! Mac's own bar does. Both colour wells are real `AXColorWell`-style
//! controls that open the same crayon grid (`render_colours_panel`),
//! targeted at text or highlight by which one was clicked
//! (`ColourTarget`) — the highlight well is not limited to the Format ▸
//! Font ▸ Highlight menu's seven named presets, which stays a separate
//! shortcut into the same underlying colour.

use super::super::format_text::ColourTarget;
use super::*;
use rmac_ui::PopUpButton;

/// A toggle segment (B/I/U/S): small, bordered, highlighted while on.
///
/// `Size::Small`'s normal 12 pt side padding (gpui-component's
/// `button.rs`) alone made eight one-glyph buttons (here and in the
/// alignment segment below) wider than the Mac's own 22 pt segment
/// buttons (design-lab/text-editor-format-bar.html), before any gap or
/// label width was even counted, which pushed the bar's right edge past
/// TextEdit's 586 pt default window and clipped it. `rmac_ui::Button`
/// has no `.compact()` the way gpui-component's own `ButtonGroup` does
/// (its `.compact()` is a different type), but it does implement `Styled`
/// (`.refine_style` applies these on top of `Size::Small`'s own padding),
/// so the same 6 pt/24 pt `.compact()` would have used are set directly.
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
        .px(px(6.0))
        .min_w(px(24.0))
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
        // The well's own swatch is the selection's real highlight colour
        // (`style.highlight`), not `highlight_swatch`'s closest-matching
        // named preset — the well (UIA-07) can now set any colour, which
        // `highlight_at_selection` has no name for.
        let highlight_colour = style.highlight.map(rich::palette::to_hsla);
        let alignment = ruler.alignment;
        // The same tighter padding as `style_toggle`'s doc comment above.
        let align_button = |id: &'static str,
                            label: &'static str,
                            accessible: &'static str,
                            value: rich::Alignment,
                            cx: &mut Context<Self>| {
            Button::new(id, label)
                .small()
                .px(px(6.0))
                .min_w(px(24.0))
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
            // B I U S. A tighter 2 pt gap between these four, matching the
            // Mac's own `.seg{gap:2px}` segment (design-lab/
            // text-editor-format-bar.html) rather than `gap_1()`'s 4 pt.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
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
            // Highlight/background colour well (UIA-07): a real `AXColorWell`
            // on the Mac, the same as the text colour well beside it — not
            // just the Format ▸ Font ▸ Highlight menu's seven named presets
            // (`HighlightNone`.../`HighlightBlue`, kept as that menu's own
            // shortcuts into the same underlying colour, independent of
            // this well). Opens the same crayon grid, targeted at highlight.
            .child(
                div()
                    .id("format-highlight-colour")
                    .role(Role::Button)
                    .aria_label(highlight_swatch.map_or("Highlight Colour", |h| h.label()))
                    .size(px(20.0))
                    .rounded(px(mac::radius_control()))
                    .border_1()
                    .border_color(mac::separator())
                    .bg(highlight_colour.unwrap_or_else(mac::control_fill))
                    .cursor_pointer()
                    .when(editable, |well| {
                        well.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.show_colours_for(ColourTarget::Highlight, cx);
                        }))
                    }),
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
            // Alignment: the same tighter 2 pt gap as the B/I/U/S segment.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
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
