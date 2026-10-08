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
//!
//! Every control has the Mac's fixed width (the x positions measured in the
//! design-lab mock), so the bar's width never depends on the UI font's
//! metrics: on Windows a wider fallback font once pushed the bar past
//! TextEdit's 586 pt window (WIN-OS-51). When the window is narrower than
//! the whole bar, the groups that do not fit move, from the right, into a »
//! overflow menu, as TextEdit's toolbar does.

use super::super::format_text::ColourTarget;
use super::*;
use crate::ShowHighlightColours;

/// The bar's side padding and the gap between its groups (the Mac's 8 pt).
const BAR_PADDING: f32 = 8.0;
const GROUP_GAP: f32 = 8.0;
/// One B/I/U/S or alignment button, and the gap inside those segments.
const SEGMENT_BUTTON: f32 = 18.0;
const SEGMENT_GAP: f32 = 2.0;
const SEGMENT: f32 = 4.0 * SEGMENT_BUTTON + 3.0 * SEGMENT_GAP;
const WELL: f32 = 20.0;
const WELL_GAP: f32 = 5.0;
const FAMILY: f32 = 77.0;
const TYPEFACE: f32 = 77.0;
const STEPPER_BUTTON: f32 = 16.0;
const STEPPER_LABEL: f32 = 15.0;
const SIZE: f32 = 2.0 * STEPPER_BUTTON + STEPPER_LABEL;
const LIST: f32 = 43.0;
const SPACING: f32 = 50.0;
/// The » button that opens the overflow menu.
const OVERFLOW: f32 = 22.0;

/// The bar's groups, left to right; the overflow menu takes them from the
/// right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Styles,
    Wells,
    Family,
    Typeface,
    Size,
    Alignment,
    List,
    Spacing,
}

const GROUPS: [Group; 8] = [
    Group::Styles,
    Group::Wells,
    Group::Family,
    Group::Typeface,
    Group::Size,
    Group::Alignment,
    Group::List,
    Group::Spacing,
];

impl Group {
    fn width(self) -> f32 {
        match self {
            Group::Styles | Group::Alignment => SEGMENT,
            Group::Wells => 2.0 * WELL + WELL_GAP,
            Group::Family => FAMILY,
            Group::Typeface => TYPEFACE,
            Group::Size => SIZE,
            Group::List => LIST,
            Group::Spacing => SPACING,
        }
    }
}

/// The bar's width with the first `shown` groups, plus the » button when
/// `overflow`.
fn bar_width(shown: usize, overflow: bool) -> f32 {
    let groups: f32 = GROUPS[..shown].iter().map(|group| group.width()).sum();
    let items = shown + usize::from(overflow);
    let gaps = items.saturating_sub(1) as f32 * GROUP_GAP;
    2.0 * BAR_PADDING + groups + gaps + if overflow { OVERFLOW } else { 0.0 }
}

/// How many groups fit in `width`; the rest go in the overflow menu. The
/// B/I/U/S segment always shows.
fn shown_groups(width: f32) -> usize {
    if !width.is_finite() || width <= 0.0 || width >= bar_width(GROUPS.len(), false) {
        return GROUPS.len();
    }
    (1..GROUPS.len())
        .rev()
        .find(|&shown| bar_width(shown, true) <= width)
        .unwrap_or(1)
}

/// A toggle segment (B/I/U/S): small, bordered, highlighted while on, the
/// Mac's 18 pt wide whatever the font.
fn style_toggle(
    id: &'static str,
    label: &'static str,
    accessible: &'static str,
    on: bool,
    cx: &mut Context<EditorView>,
    handler: impl Fn(&mut EditorView, &mut Context<EditorView>) + 'static,
) -> impl IntoElement {
    segment_button(id, label, accessible)
        .selected(on)
        .on_click(cx.listener(move |this, _, _, cx| handler(this, cx)))
}

fn segment_button(id: &'static str, label: &'static str, accessible: &'static str) -> Button {
    Button::new(id, label)
        .small()
        .flex_none()
        .w(px(SEGMENT_BUTTON))
        .min_w(px(SEGMENT_BUTTON))
        .px_0()
        .tooltip(accessible)
}

/// A bordered button of the Mac's fixed `width`; a long label is clipped
/// rather than widening the bar.
fn fixed_button(id: &'static str, label: impl Into<SharedString>, width: f32) -> Button {
    Button::new(id, label)
        .small()
        .flex_none()
        .w(px(width))
        .min_w(px(width))
        .px(px(4.0))
        .overflow_hidden()
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
    /// The bar for a window whose content is `width` points wide.
    pub(super) fn render_format_bar(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = shown_groups(width);
        let visible = |group: Group| GROUPS[..shown].contains(&group);
        let hidden: Vec<Group> = GROUPS[shown..].to_vec();
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
        let align_button = |id: &'static str,
                            label: &'static str,
                            accessible: &'static str,
                            value: rich::Alignment,
                            cx: &mut Context<Self>| {
            segment_button(id, label, accessible)
                .selected(alignment == value)
                .on_click(cx.listener(move |this, _, _, cx| this.set_alignment(value, cx)))
        };
        div()
            .id("format-bar")
            .flex_none()
            .h(px(36.0))
            .flex()
            .items_center()
            .gap(px(GROUP_GAP))
            .px(px(BAR_PADDING))
            .overflow_hidden()
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            // B I U S, with the Mac's 2 pt gap inside the segment.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(SEGMENT_GAP))
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
            // The text colour well, then the highlight/background well
            // (UIA-07): a real `AXColorWell` on the Mac, the same as the
            // text colour well beside it — not just the Format ▸ Font ▸
            // Highlight menu's seven named presets (`HighlightNone`...
            // `HighlightBlue`, kept as that menu's own shortcuts into the
            // same underlying colour). Both open the same crayon grid.
            .when(visible(Group::Wells), |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(WELL_GAP))
                        .child(
                            div()
                                .id("format-text-colour")
                                .role(Role::Button)
                                .aria_label("Text Colour")
                                .size(px(WELL))
                                .flex_none()
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
                        .child(
                            div()
                                .id("format-highlight-colour")
                                .role(Role::Button)
                                .aria_label(
                                    highlight_swatch.map_or("Highlight Colour", |h| h.label()),
                                )
                                .size(px(WELL))
                                .flex_none()
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
                        ),
                )
            })
            // Font family: opens the Fonts panel (⌘T), already a live,
            // scanned list — a second inline copy of that list would be a
            // second, divergent implementation of the same picker.
            .when(visible(Group::Family), |bar| {
                bar.child(
                    fixed_button("format-font-family", family_label, FAMILY)
                        .tooltip("Font")
                        .disabled(!editable)
                        .on_click(cx.listener(|this, _, window, cx| this.show_fonts(window, cx))),
                )
            })
            // Style (Regular/Bold/Italic/Bold Italic): a fixed four-item
            // pop-up distinct from the B/I toggles beside it.
            .when(visible(Group::Typeface), |bar| {
                bar.child(
                    fixed_button(
                        "format-typeface",
                        typeface_label(style.bold, style.italic),
                        TYPEFACE,
                    )
                    .tooltip("Typeface")
                    .disabled(!editable)
                    .dropdown_menu(|menu, _, _| {
                        menu.menu("Regular", Box::new(TypefaceRegular))
                            .menu("Bold", Box::new(TypefaceBold))
                            .menu("Italic", Box::new(TypefaceItalic))
                            .menu("Bold Italic", Box::new(TypefaceBoldItalic))
                    }),
                )
            })
            // Size: the selection's point size with a stepper, the same
            // Bigger/Smaller step Format ▸ Font already applies.
            .when(visible(Group::Size), |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .w(px(SIZE))
                        .child(
                            Button::new("format-size-down", "−")
                                .xsmall()
                                .flex_none()
                                .size(px(STEPPER_BUTTON))
                                .min_w(px(STEPPER_BUTTON))
                                .px_0()
                                .tooltip("Smaller")
                                .disabled(!editable)
                                .on_click(cx.listener(|this, _, _, cx| this.smaller(cx))),
                        )
                        .child(
                            div()
                                .w(px(STEPPER_LABEL))
                                .flex_none()
                                .overflow_hidden()
                                .text_center()
                                .text_size(rmac_ui::text_px(11.0))
                                .child(size_label),
                        )
                        .child(
                            Button::new("format-size-up", "+")
                                .xsmall()
                                .flex_none()
                                .size(px(STEPPER_BUTTON))
                                .min_w(px(STEPPER_BUTTON))
                                .px_0()
                                .tooltip("Bigger")
                                .disabled(!editable)
                                .on_click(cx.listener(|this, _, _, cx| this.bigger(cx))),
                        ),
                )
            })
            // Alignment, with the same 2 pt gap as the B/I/U/S segment.
            .when(visible(Group::Alignment), |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(SEGMENT_GAP))
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
            })
            // List: opens the List… sheet (Format ▸ List…), which already
            // covers every marker kind.
            .when(visible(Group::List), |bar| {
                bar.child(
                    fixed_button("format-list", list_label, LIST)
                        .tooltip("List")
                        .selected(ruler.list.is_some())
                        .disabled(!editable)
                        .on_click(cx.listener(|this, _, _, cx| this.open_lists(cx))),
                )
            })
            // Line spacing: opens the Spacing… sheet (Format ▸ Text ▸
            // Spacing…), which already covers every multiple.
            .when(visible(Group::Spacing), |bar| {
                bar.child(
                    fixed_button("format-spacing", spacing_label, SPACING)
                        .tooltip("Line Spacing")
                        .disabled(!editable)
                        .on_click(cx.listener(|this, _, _, cx| this.open_spacing(cx))),
                )
            })
            // » the groups that did not fit, as rows of one menu.
            .when(!hidden.is_empty(), |bar| {
                bar.child(
                    fixed_button("format-overflow", "»", OVERFLOW)
                        .px_0()
                        .tooltip("More Formatting")
                        .disabled(!editable)
                        .dropdown_menu(move |menu, _, _| {
                            let mut menu = menu;
                            for (index, group) in hidden.iter().enumerate() {
                                if index > 0 {
                                    menu = menu.separator();
                                }
                                menu = match group {
                                    Group::Styles => menu,
                                    Group::Wells => menu
                                        .menu("Text Colour…", Box::new(ShowColours))
                                        .menu("Highlight Colour…", Box::new(ShowHighlightColours)),
                                    Group::Family => menu.menu("Font…", Box::new(ShowFonts)),
                                    Group::Typeface => menu
                                        .menu("Regular", Box::new(TypefaceRegular))
                                        .menu("Bold", Box::new(TypefaceBold))
                                        .menu("Italic", Box::new(TypefaceItalic))
                                        .menu("Bold Italic", Box::new(TypefaceBoldItalic)),
                                    Group::Size => menu
                                        .menu("Bigger", Box::new(IncreaseFont))
                                        .menu("Smaller", Box::new(DecreaseFont)),
                                    Group::Alignment => menu
                                        .menu("Align Left", Box::new(AlignLeft))
                                        .menu("Centre", Box::new(AlignCentre))
                                        .menu("Align Right", Box::new(AlignRight))
                                        .menu("Justify", Box::new(AlignJustify)),
                                    Group::List => menu.menu("List…", Box::new(ShowLists)),
                                    Group::Spacing => {
                                        menu.menu("Line Spacing…", Box::new(OpenSpacing))
                                    }
                                };
                            }
                            menu
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_bar_fits_textedits_default_window() {
        // TextEdit's 586 pt document window (b37939d9), whatever the font.
        assert!(bar_width(GROUPS.len(), false) <= 586.0);
        assert_eq!(shown_groups(586.0), GROUPS.len());
    }

    #[test]
    fn a_narrow_window_moves_groups_from_the_right_into_the_overflow_menu() {
        let full = bar_width(GROUPS.len(), false);
        let shown = shown_groups(full - 1.0);
        assert!(shown < GROUPS.len());
        assert!(bar_width(shown, true) <= full - 1.0);
        // The groups that stay are the leftmost ones.
        assert_eq!(GROUPS[..shown][0], Group::Styles);
        for width in [120.0, 200.0, 320.0, 450.0, 540.0] {
            let shown = shown_groups(width);
            assert!(shown >= 1);
            assert!(shown == 1 || bar_width(shown, true) <= width, "{width}");
        }
        assert_eq!(shown_groups(10.0), 1);
    }
}
