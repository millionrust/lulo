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
        sheet_card("lists-sheet", 260.0, 190.0)
            .child(sheet_title("List"))
            .child(radio_row(
                "list-none",
                "None",
                list.is_none(),
                |this, _, cx| this.set_list(None, cx),
                cx,
            ))
            .child(radio_row(
                "list-bullets",
                "• Bullets",
                list == Some(rich::ListKind::Bullet),
                |this, _, cx| this.set_list(Some(rich::ListKind::Bullet), cx),
                cx,
            ))
            .child(radio_row(
                "list-numbers",
                "1. 2. 3. Numbers",
                list == Some(rich::ListKind::Numbered),
                |this, _, cx| this.set_list(Some(rich::ListKind::Numbered), cx),
                cx,
            ))
            .child(
                div().mt_auto().flex().justify_end().child(
                    rmac_ui::dialog_button("lists-done", "Done", DialogButtonKind::Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.close_lists(window, cx))),
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
