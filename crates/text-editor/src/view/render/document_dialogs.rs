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
        let spacing = self.ruler.line_spacing;
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

    /// Format ▸ Text ▸ Show Ruler: a thin alignment bar under the title
    /// bar, shown only in rich-text mode (TXT-MENU-071). Lulo's document
    /// model has no per-paragraph attributes, so these three buttons set
    /// the whole document's alignment at once (see `view/format_text.rs`).
    pub(super) fn render_ruler_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let alignment = self.ruler.alignment;
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
            .child(
                Button::new("ruler-align-left", "⟸")
                    .small()
                    .selected(alignment == gpui::TextAlign::Left)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_alignment(gpui::TextAlign::Left, cx)),
                    ),
            )
            .child(
                Button::new("ruler-align-centre", "≡")
                    .small()
                    .selected(alignment == gpui::TextAlign::Center)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_alignment(gpui::TextAlign::Center, cx)
                    })),
            )
            .child(
                Button::new("ruler-align-right", "⟹")
                    .small()
                    .selected(alignment == gpui::TextAlign::Right)
                    .on_click(
                        cx.listener(|this, _, _, cx| {
                            this.set_alignment(gpui::TextAlign::Right, cx)
                        }),
                    ),
            )
    }
}
