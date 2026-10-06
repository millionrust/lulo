//! File ▸ Rename…/Page Setup… and Format ▸ Text ▸ Spacing… sheets, plus the
//! Format ▸ Text ▸ Show Ruler bar (TXT-MENU-002/006/074/071).

use super::*;
use gpui::AnyElement;
use rmac_ui::DialogButtonKind;

/// A clickable radio row shared by the Page Setup and Spacing sheets.
fn radio_row(
    id: &'static str,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&mut EditorView, &mut Window, &mut Context<EditorView>) + 'static,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(label)
        .aria_selected(selected)
        .flex()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .px_2()
        .rounded(px(mac::radius_control()))
        .hover(|hovered| hovered.bg(mac::hover()))
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(if selected { "✓" } else { " " })
        .child(label)
}

fn sheet_card(id: &'static str, width: f32, height: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(width))
        .h(px(height))
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .rounded(px(mac::radius_card()))
        .bg(mac::sheet())
        .border_1()
        .border_color(mac::separator())
        .shadow_xl()
        .occlude()
}

fn sheet_title(title: &'static str) -> impl IntoElement {
    div()
        .text_size(rmac_ui::text_px(15.0))
        .font_weight(mac::BOLD)
        .child(title)
}

impl EditorView {
    pub(super) fn render_rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        sheet_card("rename-sheet", 380.0, 160.0)
            .child(sheet_title("Rename Document"))
            .child(
                div()
                    .id("rename-name")
                    .role(Role::TextInput)
                    .aria_label("Name")
                    .accessible_text_input(&self.rename_input, cx)
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                cx.stop_propagation();
                                this.cancel_rename(window, cx);
                            }
                            "enter" => {
                                cx.stop_propagation();
                                this.commit_rename(window, cx);
                            }
                            _ => {}
                        }
                    }))
                    .child(TextField::new(&self.rename_input)),
            )
            .when_some(self.rename_error.clone(), |card, message| {
                card.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::danger())
                        .child(message),
                )
            })
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button("rename-cancel", "Cancel", DialogButtonKind::Normal)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel_rename(window, cx)),
                            ),
                    )
                    .child(
                        rmac_ui::dialog_button("rename-save", "Rename", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit_rename(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_page_setup_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let letter = self.page_setup_letter;
        let landscape = self.page_setup_landscape;
        sheet_card("page-setup-sheet", 320.0, 260.0)
            .child(sheet_title("Page Setup"))
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_secondary())
                    .child("Paper Size"),
            )
            .child(radio_row(
                "page-setup-a4",
                "A4",
                !letter,
                |this, _, cx| this.set_page_setup_paper(false, cx),
                cx,
            ))
            .child(radio_row(
                "page-setup-letter",
                "US Letter",
                letter,
                |this, _, cx| this.set_page_setup_paper(true, cx),
                cx,
            ))
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_secondary())
                    .child("Orientation"),
            )
            .child(radio_row(
                "page-setup-portrait",
                "Portrait",
                !landscape,
                |this, _, cx| this.set_page_setup_orientation(false, cx),
                cx,
            ))
            .child(radio_row(
                "page-setup-landscape",
                "Landscape",
                landscape,
                |this, _, cx| this.set_page_setup_orientation(true, cx),
                cx,
            ))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "page-setup-cancel",
                            "Cancel",
                            DialogButtonKind::Normal,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.cancel_page_setup(window, cx)),
                        ),
                    )
                    .child(
                        rmac_ui::dialog_button("page-setup-ok", "OK", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.close_page_setup(window, cx)
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_spacing_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let spacing = self.paragraph_style(cx).line_spacing;
        sheet_card("spacing-sheet", 260.0, 190.0)
            .child(sheet_title("Spacing"))
            .child(radio_row(
                "spacing-single",
                "Single",
                spacing == 1.0,
                |this, _, cx| this.set_spacing(1.0, cx),
                cx,
            ))
            .child(radio_row(
                "spacing-one-half",
                "1.5 Lines",
                spacing == 1.5,
                |this, _, cx| this.set_spacing(1.5, cx),
                cx,
            ))
            .child(radio_row(
                "spacing-double",
                "Double",
                spacing == 2.0,
                |this, _, cx| this.set_spacing(2.0, cx),
                cx,
            ))
            .child(
                div().mt_auto().flex().justify_end().child(
                    rmac_ui::dialog_button("spacing-done", "Done", DialogButtonKind::Primary)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_spacing(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    /// Format ▸ Text ▸ Show Ruler: a thin bar under the title bar with the
    /// caret paragraph's alignment and list, shown only in rich-text mode
    /// (TXT-MENU-071). Each button applies to the selected paragraphs.
    pub(super) fn render_ruler_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ruler = self.paragraph_style(cx);
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
            .id("ruler-bar")
            .flex_none()
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            .child(align_button(
                "ruler-align-left",
                "⟸",
                "Align Left",
                rich::Alignment::Left,
                cx,
            ))
            .child(align_button(
                "ruler-align-centre",
                "≡",
                "Centre",
                rich::Alignment::Center,
                cx,
            ))
            .child(align_button(
                "ruler-align-justify",
                "☰",
                "Justify",
                rich::Alignment::Justified,
                cx,
            ))
            .child(align_button(
                "ruler-align-right",
                "⟹",
                "Align Right",
                rich::Alignment::Right,
                cx,
            ))
            .child(
                Button::new("ruler-list", "• List")
                    .small()
                    .tooltip("List")
                    .selected(ruler.list.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.open_lists(cx))),
            )
    }

    /// Format ▸ Font ▸ Show Colours (⇧⌘C): the Colours panel's crayon box.
    /// Clicking a crayon colours the selection (or the next typing).
    pub(super) fn render_colours_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.rich.read(cx).style_at_selection().color;
        let mut grid = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .w(px(8.0 * 28.0 + 7.0 * 4.0));
        for (name, color) in rich::palette::CRAYONS {
            let selected = current == Some(color);
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("crayon-{name}")))
                    .role(Role::RadioButton)
                    .aria_label(name)
                    .aria_selected(selected)
                    .w(px(28.0))
                    .h(px(20.0))
                    .rounded(px(mac::radius_control()))
                    .bg(rich::palette::to_hsla(color))
                    .border_1()
                    .border_color(if selected {
                        mac::system_blue()
                    } else {
                        mac::separator()
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_text_colour(Some(color), cx)),
                    ),
            );
        }
        sheet_card("colours-sheet", 296.0, 300.0)
            .child(sheet_title("Colours"))
            .child(grid)
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_between()
                    .child(
                        rmac_ui::dialog_button(
                            "colours-automatic",
                            "Automatic",
                            DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.set_text_colour(None, cx))),
                    )
                    .child(
                        rmac_ui::dialog_button("colours-done", "Done", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_colours(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Format ▸ List…: bullets, numbers, or no list for the selected
    /// paragraphs.
    pub(super) fn render_lists_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let list = self.paragraph_style(cx).list;
        // None, then every NSTextList marker (design-lab/
        // text-editor-format-sheets.html).
        let choices = std::iter::once(None).chain(rich::ListKind::ALL.into_iter().map(Some));
        let mut grid = div()
            .id("list-markers")
            .role(Role::RadioGroup)
            .aria_label("Bullet")
            .flex()
            .flex_wrap()
            .gap(px(4.0));
        for (index, choice) in choices.enumerate() {
            let selected = list == choice;
            let (label, accessible): (SharedString, SharedString) = match choice {
                None => ("None".into(), "None".into()),
                Some(kind) => (
                    kind.marker(1).into(),
                    super::super::format_extras::list_marker_name(kind).into(),
                ),
            };
            grid = grid.child(
                div()
                    .id(("list-marker", index))
                    .role(Role::RadioButton)
                    .aria_label(accessible)
                    .aria_selected(selected)
                    .w(px(42.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(mac::radius_control()))
                    .bg(if selected {
                        mac::accent()
                    } else {
                        mac::control_fill()
                    })
                    .text_color(if selected { mac::white() } else { mac::text() })
                    .hover(|row| row.opacity(0.85))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_list(choice, cx))),
            );
        }
        sheet_card("lists-sheet", 320.0, 250.0)
            .child(sheet_title("List"))
            .child(div().text_color(mac::text_secondary()).child("Bullet:"))
            .child(grid)
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child("Press Tab at the start of an item to nest it, ⇧Tab to bring it out."),
            )
            .child(
                div().mt_auto().flex().justify_end().child(
                    rmac_ui::dialog_button("lists-done", "Done", DialogButtonKind::Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.close_lists(window, cx))),
                ),
            )
            .into_any_element()
    }

    /// File ▸ Show Properties (⌥⌘P): TextEdit's seven document properties
    /// (design-lab/text-editor-format-sheets.html). OK keeps them in the
    /// document (saved with rich text); Cancel and Esc leave it unchanged.
    pub(super) fn render_properties_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = sheet_card("properties-sheet", 420.0, 330.0)
            .gap(px(8.0))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        cx.stop_propagation();
                        this.close_properties(window, cx);
                    }
                    "enter" => {
                        cx.stop_propagation();
                        this.commit_properties(window, cx);
                    }
                    _ => {}
                }
            }))
            .child(sheet_title("Document Properties"));
        for (index, (label, input)) in super::super::format_extras::PROPERTY_FIELDS
            .iter()
            .zip(&self.property_inputs)
            .enumerate()
        {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(96.0))
                            .flex()
                            .justify_end()
                            .text_color(mac::text_secondary())
                            .child(*label),
                    )
                    .child(
                        div()
                            .id(("property-field", index))
                            .flex_1()
                            .role(Role::TextInput)
                            .aria_label(label.trim_end_matches(':'))
                            .accessible_text_input(input, cx)
                            .child(TextField::new(input)),
                    ),
            );
        }
        card.child(
            div()
                .mt_auto()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(
                    rmac_ui::dialog_button("properties-cancel", "Cancel", DialogButtonKind::Normal)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_properties(window, cx)),
                        ),
                )
                .child(
                    rmac_ui::dialog_button("properties-ok", "OK", DialogButtonKind::Primary)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.commit_properties(window, cx)),
                        ),
                ),
        )
        .into_any_element()
    }

    /// Format ▸ Font ▸ Styles…: the document's styles one at a time, each
    /// previewed in itself; Apply gives it to the selection.
    pub(super) fn render_styles_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let styles = self.document_styles(cx);
        let count = styles.len();
        let index = self.styles_index.min(count.saturating_sub(1));
        let preview = styles.get(index).map(|(style, ruler)| {
            let description = super::super::format_extras::style_description(style, ruler);
            let mut sample = div()
                .id("styles-preview-text")
                .role(Role::Label)
                .aria_label(description.clone())
                .text_size(px(style.size.clamp(9.0, 28.0)))
                .text_color(
                    style
                        .color
                        .map_or(mac::text(), |color| gpui::rgb(color.to_u32()).into()),
                );
            if let Some(family) = &style.family {
                sample = sample.font_family(SharedString::from(family.to_string()));
            }
            if style.bold {
                sample = sample.font_weight(mac::BOLD);
            }
            if style.italic {
                sample = sample.italic();
            }
            if style.underline {
                sample = sample.underline();
            }
            if style.strikethrough {
                sample = sample.line_through();
            }
            if let Some(highlight) = style.highlight {
                sample = sample.bg(gpui::Hsla::from(gpui::rgb(highlight.to_u32())));
            }
            sample.child(description)
        });
        sheet_card("styles-sheet", 380.0, 220.0)
            .child(sheet_title("Styles"))
            .child(
                div()
                    .text_color(mac::text_secondary())
                    .child("Document styles"),
            )
            .child(
                div()
                    .id("styles-preview")
                    .h(px(64.0))
                    .p(px(8.0))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .rounded(px(mac::radius_control()))
                    .bg(mac::field_fill())
                    .children(preview),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new("styles-previous", "‹")
                            .small()
                            .tooltip("Previous Style")
                            .on_click(cx.listener(|this, _, _, cx| this.step_style(false, cx))),
                    )
                    .child(
                        div()
                            .text_color(mac::text_secondary())
                            .child(format!("{} of {count}", index + 1)),
                    )
                    .child(
                        Button::new("styles-next", "›")
                            .small()
                            .tooltip("Next Style")
                            .on_click(cx.listener(|this, _, _, cx| this.step_style(true, cx))),
                    ),
            )
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button("styles-apply", "Apply", DialogButtonKind::Normal)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_document_style(cx))),
                    )
                    .child(
                        rmac_ui::dialog_button("styles-done", "Done", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_styles(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Edit ▸ Link… (⌘K): the selection's link destination; Remove Link
    /// takes the link off, keeping the text.
    pub(super) fn render_link_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        sheet_card("link-sheet", 380.0, 150.0)
            .child(sheet_title("Link"))
            .child(
                div()
                    .id("link-destination")
                    .role(Role::TextInput)
                    .aria_label("Link destination")
                    .accessible_text_input(&self.link_input, cx)
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.close_link(window, cx);
                        }
                    }))
                    .child(TextField::new(&self.link_input)),
            )
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "link-remove",
                            "Remove Link",
                            DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.remove_link(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        rmac_ui::dialog_button("link-cancel", "Cancel", DialogButtonKind::Normal)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_link(window, cx)),
                            ),
                    )
                    .child(
                        rmac_ui::dialog_button("link-ok", "OK", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit_link(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Format ▸ Font ▸ Show Fonts (⌘T): the installed families (click one to
    /// set the selection's family) and a size field (Return applies it).
    pub(super) fn render_fonts_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.rich.read(cx).style_at_selection().family;
        let mut list = div()
            .id("font-families")
            .role(Role::List)
            .aria_label("Family")
            .h(px(220.0))
            .overflow_y_scroll()
            .border_1()
            .border_color(mac::separator())
            .rounded(px(mac::radius_control()));
        match &self.font_families {
            None => {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_color(mac::text_secondary())
                        .child("Loading fonts…"),
                );
            }
            Some(families) => {
                for (index, family) in families.iter().enumerate() {
                    let selected = current.as_deref() == Some(family.as_ref());
                    let family = family.clone();
                    list = list.child(
                        div()
                            .id(("font-family", index))
                            .role(Role::RadioButton)
                            .aria_label(family.clone())
                            .aria_selected(selected)
                            .h(px(22.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .bg(if selected {
                                mac::text_selection()
                            } else {
                                gpui::transparent_black()
                            })
                            .hover(|row| row.bg(mac::hover()))
                            .font_family(family.clone())
                            .child(family.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_font_family(family.clone(), cx)
                            })),
                    );
                }
            }
        }
        sheet_card("fonts-sheet", 320.0, 360.0)
            .child(sheet_title("Fonts"))
            .child(list)
            .child(
                div().flex().items_center().gap_2().child("Size").child(
                    div()
                        .id("font-size")
                        .role(Role::TextInput)
                        .aria_label("Size")
                        .accessible_text_input(&self.font_size_input, cx)
                        .w(px(80.0))
                        .child(TextField::new(&self.font_size_input)),
                ),
            )
            .child(
                div().mt_auto().flex().justify_end().child(
                    rmac_ui::dialog_button("fonts-done", "Done", DialogButtonKind::Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.close_fonts(window, cx))),
                ),
            )
            .into_any_element()
    }
}
